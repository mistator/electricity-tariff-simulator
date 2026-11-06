use crate::tariffs::Tariff;
use crate::utils::DateHour;
use chrono::{Datelike, Weekday};
use ordered_float::NotNan;

#[derive(PartialEq, Debug)]
pub enum TD20EnergyPeriod {
    Peak,
    Plain,
    Valley,
}

impl TD20EnergyPeriod {
    pub fn from_datetime(datetime: &DateHour) -> Self {
        if datetime.weekday() == Weekday::Sun || datetime.weekday() == Weekday::Sat {
            Self::Valley
        } else if datetime.hour() < 8 {
            Self::Valley
        } else if datetime.hour() < 10 {
            Self::Plain
        } else if datetime.hour() < 14 {
            Self::Peak
        } else if datetime.hour() < 18 {
            Self::Plain
        } else if datetime.hour() < 22 {
            Self::Peak
        } else {
            Self::Plain
        }
    }
}

pub enum TD20PowerPeriod {
    Peak,
    Valley
}

impl TD20PowerPeriod {
    pub fn from_datetime(datetime: &DateHour) -> Self {
        if datetime.weekday() == Weekday::Sat || datetime.weekday() == Weekday::Sun {
            Self::Valley
        } else if datetime.hour() < 8 {
            Self::Valley
        } else {
            Self::Peak
        }
    }
}

pub struct TD20 {
    pub power_valley: NotNan<f64>,
    pub power_peak: NotNan<f64>,

    pub energy_valley: NotNan<f64>,
    pub energy_plain: NotNan<f64>,
    pub energy_peak: NotNan<f64>,

    pub energy_export: NotNan<f64>
}

impl Tariff for TD20 {
    fn get_import_rate(&self, datetime: &DateHour) -> NotNan<f64> {
        let period = TD20EnergyPeriod::from_datetime(datetime);
        match period {
            TD20EnergyPeriod::Peak => self.energy_peak,
            TD20EnergyPeriod::Plain => self.energy_plain,
            TD20EnergyPeriod::Valley => self.energy_valley,
        }
    }

    fn get_export_rate(&self, _datetime: &DateHour) -> NotNan<f64> {
        self.energy_export
    }

    fn get_daily_power_rate_peak(&self) -> NotNan<f64> {
        self.power_peak
    }

    fn get_daily_power_rate_valley(&self) -> NotNan<f64> {
        self.power_valley
    }
}

