#[derive(Default)]
pub struct Avg {
    pub value: f64,
    k: usize,
}

impl Avg {
    pub fn new() -> Self {
        Self { value: 0., k: 0 }
    }

    pub fn with_initial_value(value: f64) -> Self {
        Self { value, k: 1 }
    }

    pub fn add_value(&mut self, value: f64) -> () {
        if self.k == 0 {
            self.value = value;
            self.k += 1;
        } else {
            self.value = (self.k as f64 / (self.k as f64 + 1.)) * (self.value + (value / self.k as f64));
            self.k += 1;
        }
    }
}

#[derive(Default)]
pub struct AvgDailyConsumption {
    pub peak: Avg,
    pub plain: Avg,
    pub valley: Avg,
}

#[allow(unused)]
#[derive(Default)]
pub struct AvgHourlyConsumption([Avg; 24]);

#[derive(Default)]
pub struct AvgWeeklyConsumption {
    pub monday: AvgDailyConsumption,
    pub tuesday: AvgDailyConsumption,
    pub wednesday: AvgDailyConsumption,
    pub thursday: AvgDailyConsumption,
    pub friday: AvgDailyConsumption,
    pub saturday: AvgDailyConsumption,
    pub sunday: AvgDailyConsumption,
}

#[derive(Default)]
pub struct AvgMonthlyConsumption {
    pub jan: AvgWeeklyConsumption,
    pub feb: AvgWeeklyConsumption,
    pub mar: AvgWeeklyConsumption,
    pub apr: AvgWeeklyConsumption,
    pub may: AvgWeeklyConsumption,
    pub jun: AvgWeeklyConsumption,
    pub jul: AvgWeeklyConsumption,
    pub aug: AvgWeeklyConsumption,
    pub sep: AvgWeeklyConsumption,
    pub oct: AvgWeeklyConsumption,
    pub nov: AvgWeeklyConsumption,
    pub dec: AvgWeeklyConsumption,
}

pub struct Analysis {

}
