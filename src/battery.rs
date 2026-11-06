use crate::tariffs::td20::TD20EnergyPeriod;
use crate::utils::{DateHour, EnergyPricesSlice};
use crate::Installation;
use chrono::Days;
use num_traits::{One, Zero};
use ordered_float::NotNan;

pub struct BatteryDescription {
    pub capacity: NotNan<f64>,
    pub discharge_range: (NotNan<f64>, NotNan<f64>),
    pub strategy: Box<dyn BatteryStrategy>,
}

pub struct Battery {
    capacity: NotNan<f64>,
    discharge_range: (NotNan<f64>, NotNan<f64>),
    current_charge: NotNan<f64>,
}

impl Battery {
    pub fn from(battery: &BatteryDescription) -> Self {
        Self {
            capacity: battery.capacity,
            discharge_range: battery.discharge_range,
            current_charge: NotNan::zero(),
        }
    }

    pub fn none() -> Self {
        Self {
            capacity: NotNan::zero(),
            discharge_range: (NotNan::zero(), NotNan::one()),
            current_charge: NotNan::zero(),
        }
    }

    pub fn available_capacity(&self) -> NotNan<f64> {
        let max_charge = self.capacity * self.discharge_range.1;
        if self.current_charge > max_charge {
            NotNan::zero()
        } else {
            max_charge - self.current_charge
        }
    }

    pub fn available_charge(&self) -> NotNan<f64> {
        self.charge_over(self.discharge_range.0)
    }

    pub fn charge_over(&self, percentage: NotNan<f64>) -> NotNan<f64> {
        let min_charge = self.capacity * percentage;
        if self.current_charge < min_charge {
            NotNan::zero()
        } else {
            self.current_charge - min_charge
        }
    }

    pub fn capacity_under(&self, percentage: NotNan<f64>) -> NotNan<f64> {
        let max_charge = self.capacity * percentage;
        if self.current_charge > max_charge {
            NotNan::zero()
        } else {
            max_charge - self.current_charge
        }
    }
}

pub struct BatteryCtx<'a> {
    pub installation: &'a Installation,
    pub datetime: &'a DateHour,
    pub today_prices: &'a EnergyPricesSlice,
    pub tomorrow_prices: &'a EnergyPricesSlice,
}

pub trait BatteryStrategy {
    fn do_strategy(&mut self, consumption: &mut NotNan<f64>, battery: &mut Battery, ctx: &BatteryCtx);

    fn reset(&mut self) {}

    fn charge_solar_surplus(&self, consumption: &mut NotNan<f64>, battery: &mut Battery) {
        let available_capacity = battery.available_capacity();
        let added_charge = NotNan::min(available_capacity, -*consumption);

        battery.current_charge += added_charge;

        *consumption += added_charge
    }

    fn discharge_consumption(&self, consumption: &mut NotNan<f64>, battery: &mut Battery) {
        let available_charge = battery.available_charge();
        let discharge = NotNan::min(available_charge, *consumption);

        battery.current_charge -= discharge;
        *consumption -= discharge;
    }

    fn discharge_full(&self, consumption: &mut NotNan<f64>, battery: &mut Battery, limit: NotNan<f64>) {
        let limit = NotNan::max(battery.discharge_range.0, limit);

        let available_charge = battery.charge_over(limit);
        let discharge = NotNan::min(available_charge, battery.capacity / NotNan::from(2u8));

        battery.current_charge -= discharge;
        *consumption -= discharge;
    }

    fn charge_from_grid(
        &self,
        consumption: &mut NotNan<f64>,
        battery: &mut Battery,
        limit: NotNan<f64>,
        ctx: &BatteryCtx,
    ) {
        let power = ctx.installation.power_at_datetime(ctx.datetime);

        let available_capacity = battery.capacity_under(limit);
        let added_charge = NotNan::min(
            available_capacity,
            NotNan::min(power - *consumption, battery.capacity / NotNan::from(2u8)));

        battery.current_charge += added_charge;

        *consumption += added_charge
    }
}

pub struct BatteryDoNothing {}

impl BatteryStrategy for BatteryDoNothing {
    fn do_strategy(
        &mut self,
        _: &mut NotNan<f64>,
        _: &mut Battery,
        _: &BatteryCtx
    ) {}
}

pub struct ChargeAlwaysDischargePeak {}

impl BatteryStrategy for ChargeAlwaysDischargePeak {
    fn do_strategy(&mut self, consumption: &mut NotNan<f64>, battery: &mut Battery, ctx: &BatteryCtx) {
        if *consumption < NotNan::zero() {
            self.charge_solar_surplus(consumption, battery);
        } else {
            let period = TD20EnergyPeriod::from_datetime(ctx.datetime);

            if period == TD20EnergyPeriod::Peak {
                self.discharge_consumption(consumption, battery);
            }
        }
    }
}


pub struct ChargeSolarAndNightDischargePeak {}

impl BatteryStrategy for ChargeSolarAndNightDischargePeak {
    fn do_strategy(&mut self, consumption: &mut NotNan<f64>, battery: &mut Battery, ctx: &BatteryCtx) {
        if *consumption < NotNan::zero() {
            self.charge_solar_surplus(consumption, battery);
        } else {
            let period = TD20EnergyPeriod::from_datetime(ctx.datetime);

            if period == TD20EnergyPeriod::Peak {
                self.discharge_consumption(consumption, battery);
            } else if period == TD20EnergyPeriod::Valley {
                self.charge_from_grid(consumption, battery, NotNan::one(), ctx);
            }
        }
    }
}

pub struct ChargeSolarDischargeTopHours {
    pub n_hours: u8
}

impl BatteryStrategy for ChargeSolarDischargeTopHours {
    fn do_strategy(&mut self, consumption: &mut NotNan<f64>, battery: &mut Battery, ctx: &BatteryCtx) {
        if *consumption < NotNan::zero() {
            self.charge_solar_surplus(consumption, battery);
        } else {
            if ctx.today_prices.is_top_n_hours_export(self.n_hours, ctx.datetime.hour()) {
                self.discharge_consumption(consumption, battery);
            }
        }
    }
}

pub struct SellAtTop {
    baseload: NotNan<f64>,
    today_exports: NotNan<f64>,
}

impl SellAtTop {
    pub fn new(baseload: NotNan<f64>) -> Self {
        SellAtTop {
            baseload,
            today_exports: NotNan::zero(),
        }
    }

    fn discharge(
        &self,
        prices: &EnergyPricesSlice,
        consumption: &mut NotNan<f64>,
        battery: &mut Battery,
        reserve: NotNan<f64>,
        ctx: &BatteryCtx
    ) {
        let records = prices.top_n_hours_export(2);
        let hours = records.iter().map(|value| value.0).collect::<Vec<_>>();

        if ctx.datetime.hour() == hours[0] {
            // Top price, discharge full
            self.discharge_full(consumption, battery, reserve / battery.capacity);
        } else if ctx.datetime.hour() == hours[1] {
            // Second top price, discharge to half
            self.discharge_full(consumption, battery, NotNan::new(0.5).unwrap() + reserve / battery.capacity);
        }
    }

    fn charge(
        &self,
        prices: &EnergyPricesSlice,
        consumption: &mut NotNan<f64>,
        battery: &mut Battery,
        ctx: &BatteryCtx
    ) {
        let records = prices.bottom_n_hours_import(2);
        let hours = records.iter().map(|value| value.0).collect::<Vec<_>>();

        if ctx.datetime.hour() == hours[1] && hours[1] < hours[0] {
            // Second bottom price comes first, charge to half in first hour
            self.charge_from_grid(consumption, battery, NotNan::new(0.5).unwrap(), ctx);
        } else if ctx.datetime.hour() == hours[0] || ctx.datetime.hour() == hours[1] {
            // First top price comes first, or it's second hour
            self.charge_from_grid(consumption, battery, NotNan::one(), ctx);
        }
    }

    fn should_arbitrage(&self, prices: &EnergyPricesSlice) -> bool {
        prices.spread() > NotNan::try_from(0.05).unwrap() &&
            prices.min_import_hour() < prices.max_export_hour()
    }

    fn evening_baseload_preserve(&self, evening_prices: &EnergyPricesSlice, night_prices: &EnergyPricesSlice) -> NotNan<f64> {
        let mut baseload = NotNan::zero();

        let hour = evening_prices.top_n_hours_export(2)
            .iter()
            .map(|value| value.0)
            .max()
            .unwrap();

        baseload += self.baseload * NotNan::from(23 - hour);

        let hour = if self.should_arbitrage(night_prices) {
            night_prices.bottom_n_hours_import(2)
                .iter()
                .map(|value| value.0)
                .min()
                .unwrap()
        } else {
            9
        };

        baseload += self.baseload * NotNan::from(hour);

        baseload
    }
}

impl BatteryStrategy for SellAtTop {
    fn do_strategy(&mut self, consumption: &mut NotNan<f64>, battery: &mut Battery, ctx: &BatteryCtx) {
        let night_prices = ctx.today_prices.records
            .iter()
            .filter(|(hour, _)| *hour < 10)
            .collect::<EnergyPricesSlice>();
        let day_prices = ctx.today_prices.records
            .iter()
            .filter(|(hour, _)| *hour >= 10)
            .collect::<EnergyPricesSlice>();
        let evening_prices = ctx.today_prices.records
            .iter()
            .filter(|(hour, _)| *hour >= 18)
            .collect::<EnergyPricesSlice>();
        let following_night_prices = ctx.tomorrow_prices.records
            .iter()
            .filter(|(hour, _)| *hour < 10)
            .collect::<EnergyPricesSlice>();

        if ctx.datetime.hour() == 0 {
            let date = ctx.datetime.date().checked_sub_days(Days::new(1)).unwrap();
            println!("{}: {} KWh", date, self.today_exports);
            self.today_exports = NotNan::zero();
        }

        if *consumption < NotNan::zero() {
            self.charge_solar_surplus(consumption, battery);
        }

        if ctx.datetime.hour() < 10 {
            if self.should_arbitrage(&night_prices) {
                if night_prices.is_bottom_n_hours_import(2, ctx.datetime.hour()) {
                    self.charge(&night_prices, consumption, battery, ctx);
                } else if night_prices.is_top_n_hours_export(2, ctx.datetime.hour()) {
                    self.discharge(&night_prices, consumption, battery, NotNan::zero(), ctx);
                } else if night_prices.import_price_at(ctx.datetime.hour()) > night_prices.max_export() {
                    self.discharge_consumption(consumption, battery);
                }
            } else {
                self.discharge_consumption(consumption, battery);
            }
        } else {
            if self.should_arbitrage(&day_prices) &&
                    day_prices.is_bottom_n_hours_import(2, ctx.datetime.hour()) {
                self.charge(&day_prices, consumption, battery, ctx);
            }

            if day_prices.is_top_n_hours_export(2, ctx.datetime.hour()) {
                let baseload = self.evening_baseload_preserve(&evening_prices, &following_night_prices);
                self.discharge(&evening_prices, consumption, battery, baseload, ctx);
            } else if day_prices.import_price_at(ctx.datetime.hour()) > day_prices.max_export() {
                self.discharge_consumption(consumption, battery);
            }
        }

        if *consumption < NotNan::zero() {
            self.today_exports -= *consumption;
        }

    }

    fn reset(&mut self) {
        self.today_exports = NotNan::zero()
    }
}
