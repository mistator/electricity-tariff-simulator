use crate::tariffs::td20::TD20EnergyPeriod;
use crate::utils::{DateHour, EnergyPrices, EnergyPricesSlice};
use crate::Installation;
use chrono::Datelike;
use chrono::{Days, Weekday};
use num_traits::identities::Zero;
use ordered_float::NotNan;
use std::fmt::{Display, Formatter};

pub mod td20;
pub mod indexed;

#[derive(Default)]
pub struct PeriodEnergyCosts {
    pub days: u32,

    pub energy_valley: NotNan<f64>,
    pub energy_cost_valley: NotNan<f64>,
    pub energy_plain: NotNan<f64>,
    pub energy_cost_plain: NotNan<f64>,
    pub energy_peak: NotNan<f64>,
    pub energy_cost_peak: NotNan<f64>,
    pub energy_exports: NotNan<f64>,
    pub energy_exports_revenue: NotNan<f64>,
}

impl PeriodEnergyCosts {
    pub fn add_day(&mut self) {
        self.days += 1;
    }

    pub fn add_energy_import(&mut self, consumption: NotNan<f64>, datetime: &DateHour, rate: NotNan<f64>) {
        match TD20EnergyPeriod::from_datetime(datetime) {
            TD20EnergyPeriod::Peak => {
                self.energy_peak += consumption;
                self.energy_cost_peak += consumption * rate;
            }
            TD20EnergyPeriod::Plain => {
                self.energy_plain += consumption;
                self.energy_cost_plain += consumption * rate;
            }
            TD20EnergyPeriod::Valley => {
                self.energy_valley += consumption;
                self.energy_cost_valley += consumption * rate;
            }
        }
    }

    pub fn add_energy_export(&mut self, exports: NotNan<f64>, rate: NotNan<f64>) {
        if rate < NotNan::zero() {
            return;
        }

        self.energy_exports += exports;
        self.energy_exports_revenue += exports * rate;
    }
}

pub const IEE_RATE: NotNan<f64> = unsafe { NotNan::new_unchecked(1.0511269632) }; // %
pub const VAT_RATE: NotNan<f64> = unsafe { NotNan::new_unchecked(1.21) }; // %
pub const METER_LEASE_RATE: NotNan<f64> = unsafe { NotNan::new_unchecked( 0.8 / 30. ) }; // EUR/day
pub const SOCIAL_SUBSIDY_RATE: NotNan<f64> = unsafe { NotNan::new_unchecked(0.0246885) }; // EUR/day

#[derive(Debug)]
pub struct Bill {
    pub days: u32,

    pub energy_valley: NotNan<f64>,
    pub energy_cost_valley: NotNan<f64>,
    pub energy_plain: NotNan<f64>,
    pub energy_cost_plain: NotNan<f64>,
    pub energy_peak: NotNan<f64>,
    pub energy_cost_peak: NotNan<f64>,
    pub energy_exports: NotNan<f64>,
    pub energy_exports_revenue: NotNan<f64>,

    pub power_cost_valley: NotNan<f64>,
    pub power_cost_peak: NotNan<f64>,

    pub meter_rent_cost: NotNan<f64>,
    pub social_subsidy_cost: NotNan<f64>,

    pub total_electricity_gross_cost: NotNan<f64>,
    pub other_concepts_cost: NotNan<f64>,

    pub total_net_cost: NotNan<f64>,
}

impl Display for Bill {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bill: {} days\n", self.days)?;
        write!(f, "\tPOWER\n")?;
        write!(f, "\t===============\n")?;
        write!(f, "\tPeak:                \t{:8.2} EUR\n", self.power_cost_peak)?;
        write!(f, "\tValley:              \t{:8.2} EUR\n", self.power_cost_valley)?;
        write!(f, "\tENERGY\n")?;
        write!(f, "\t===============\n")?;
        write!(f, "\tPeak:   {:8.2} KWh;\t{:8.2} EUR\n", self.energy_peak, self.energy_cost_peak)?;
        write!(f, "\tPlain:  {:8.2} KWh;\t{:8.2} EUR\n", self.energy_plain, self.energy_cost_plain)?;
        write!(f, "\tValley: {:8.2} KWh;\t{:8.2} EUR\n", self.energy_valley, self.energy_cost_valley)?;
        write!(f, "\tExprt.: {:8.2} KWh;\t{:8.2} EUR\n", self.energy_exports, -self.energy_exports_revenue)?;
        write!(f, "")?;
        write!(f, "\tTOTAL ELECTRICITY COST (gross):\t{:8.2} EUR\n\n", self.total_electricity_gross_cost)?;
        write!(f, "\tOTHER CONCEPTS\n")?;
        write!(f, "\t===============\n")?;
        write!(f, "\tMeter rent:          \t{:8.2} EUR\n", self.meter_rent_cost)?;
        write!(f, "\tSocial subsidy cost: \t{:8.2} EUR\n", self.social_subsidy_cost)?;
        write!(f, "\tOther costs:         \t{:8.2} EUR\n\n", self.other_concepts_cost)?;
        write!(f, "\tTOTAL (net):\t{:8.2} EUR", self.total_net_cost)
    }
}

pub trait Tariff {
    fn get_import_rate(&self, datetime: &DateHour) -> NotNan<f64>;
    fn get_export_rate(&self, datetime: &DateHour) -> NotNan<f64>;

    fn get_daily_power_rate_peak(&self) -> NotNan<f64>;
    fn get_daily_power_rate_valley(&self) -> NotNan<f64>;

    fn get_other_concepts_cost(&self, _days: u32) -> NotNan<f64> { NotNan::zero() }

    fn compute_bill(&self, costs: &PeriodEnergyCosts, installation: &Installation) -> Bill {
        let days = NotNan::from(costs.days);

        let power_cost_valley = self.get_daily_power_rate_valley() * days * installation.power_valley;
        let power_cost_peak = self.get_daily_power_rate_peak() * days * installation.power_peak;
        let total_electricity_gross_cost = costs.energy_cost_peak +
            costs.energy_cost_plain +
            costs.energy_cost_valley -
            costs.energy_exports_revenue +
            power_cost_peak +
            power_cost_valley;
        let social_subsidy_cost = SOCIAL_SUBSIDY_RATE * days;
        let meter_rent_cost = METER_LEASE_RATE * days;
        let other_concepts_cost = self.get_other_concepts_cost(costs.days);

        Bill {
            days: costs.days,
            energy_valley: costs.energy_valley,
            energy_cost_valley: costs.energy_cost_valley,
            energy_plain: costs.energy_plain,
            energy_cost_plain: costs.energy_cost_plain,
            energy_peak: costs.energy_peak,
            energy_cost_peak: costs.energy_cost_peak,
            energy_exports: costs.energy_exports,
            energy_exports_revenue: costs.energy_exports_revenue,
            power_cost_valley,
            power_cost_peak,
            meter_rent_cost,
            social_subsidy_cost,
            total_electricity_gross_cost,
            other_concepts_cost,
            total_net_cost: (((
                total_electricity_gross_cost + social_subsidy_cost) * IEE_RATE) +
                meter_rent_cost + other_concepts_cost) * VAT_RATE
        }
    }

    fn update_costs(&self, consumption: NotNan<f64>, datetime: &DateHour, running_costs: &mut PeriodEnergyCosts) {
        if consumption > NotNan::zero() {
            running_costs.add_energy_import(consumption, datetime, self.get_import_rate(&datetime));
        } else {
            running_costs.add_energy_export(-consumption, self.get_export_rate(&datetime));
        };
    }

    fn get_tomorrow_prices(&self, date: &DateHour) -> EnergyPricesSlice {
        self.get_today_prices(&date.checked_add_days(Days::new(1)).unwrap())
    }

    fn get_today_prices(&self, date: &DateHour) -> EnergyPricesSlice {
        let mut date = (*date).clone();
        let mut prices = Vec::new();

        // last Sunday of October
        let is_hour_change_day_october = || {
            date.month() == 10 &&
                date.weekday() == Weekday::Sun &&
                date.checked_add_days(Days::new(7)).unwrap().month() == 11
        };

        // last Sunday of March
        let is_hour_change_day_march = || {
            date.month() == 3 &&
                date.weekday() == Weekday::Sun &&
                date.checked_add_days(Days::new(7)).unwrap().month() == 4
        };

        let n_hours = if is_hour_change_day_october() {
            25
        } else if is_hour_change_day_march() {
            23
        } else {
            24
        };

        for hour in 0..n_hours {
            date = date.with_hour(hour);

            prices.push((hour, EnergyPrices {
                import: self.get_import_rate(&date),
                export: self.get_export_rate(&date),
            }))
        }

        EnergyPricesSlice::new(prices)
    }
}
