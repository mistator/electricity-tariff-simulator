pub mod utils;
pub mod types;
pub mod tariffs;
pub mod battery;
pub mod analysis;
pub mod distributors;

use crate::battery::{Battery, BatteryCtx, BatteryDescription, BatteryStrategy, SellAtTop};
use crate::tariffs::indexed::{PVPC, TD20Indexed};
use crate::tariffs::td20::{TD20, TD20PowerPeriod};
use crate::tariffs::{Bill, PeriodEnergyCosts, Tariff};
use crate::types::ConsumptionRecords;
use crate::utils::{DateHour, PROJECT_DIRS};
use chrono::NaiveDate;
use clap::Parser;
use num_traits::Zero;
use ordered_float::NotNan;
use std::error::Error;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use zip::ZipArchive;

pub struct InstallationDescription {
    pub power_peak: NotNan<f64>,
    pub power_valley: NotNan<f64>,
    pub battery: Option<BatteryDescription>,
}

#[derive(Clone, Copy)]
pub struct Installation {
    pub power_peak: NotNan<f64>,
    pub power_valley: NotNan<f64>,
}

impl Installation {
    pub fn from(desc: &InstallationDescription) -> Self {
        Self {
            power_peak: desc.power_peak,
            power_valley: desc.power_valley
        }
    }

    pub fn power_at_datetime(&self, datetime: &DateHour) -> NotNan<f64> {
        match TD20PowerPeriod::from_datetime(datetime) {
            TD20PowerPeriod::Peak => self.power_peak,
            TD20PowerPeriod::Valley => self.power_valley,
        }
    }
}

pub struct Simulation<'a> {
    pub installation: Installation,
    pub tariff: &'a dyn Tariff,
    battery: &'a mut Option<(Battery, Box<dyn BatteryStrategy>)>,
}

impl<'a> Simulation<'a> {
    pub fn simulate(
        &mut self,
        records: &ConsumptionRecords,
        start_date: &NaiveDate,
        end_date: &NaiveDate,
    ) -> Bill {
        let mut costs = PeriodEnergyCosts::default();

        let daily_records_set = records
            .iter()
            .filter(|(date, _)| date.date() >= *start_date && date.date() <= *end_date)
            .collect::<ConsumptionRecords>();
        let daily_records_set = daily_records_set
            .by_day();

        for daily_records in daily_records_set {
            costs.add_day();

            let datetime = daily_records.first().unwrap().0;

            let today_prices = self.tariff.get_today_prices(&datetime);
            let tomorrow_prices = self.tariff.get_tomorrow_prices(&datetime);

            for (datetime, consumption) in daily_records.iter() {
                let mut consumption = *consumption;

                if let Some((battery, strategy)) = self.battery {
                    strategy.do_strategy(&mut consumption, battery, &BatteryCtx {
                        installation: &self.installation,
                        datetime: &datetime,
                        today_prices: &today_prices,
                        tomorrow_prices: &tomorrow_prices,
                    });
                }


                self.tariff.update_costs(consumption, datetime, &mut costs);
            }
        }

        self.tariff.compute_bill(&costs, &self.installation)
    }
}

pub struct Comparison<'a> {
    original_installation: Installation,
    original_tariff: &'a dyn Tariff,
    original_battery: Option<(Battery, Box<dyn BatteryStrategy>)>,

    new_installation: Installation,
    new_battery: Option<(Battery, Box<dyn BatteryStrategy>)>,

    compare_tariffs: Vec<&'a dyn Tariff>
}

impl<'a> Comparison<'a> {
    pub fn new(
        original_installation: InstallationDescription,
        original_tariff: &'a dyn Tariff,
        new_installation: InstallationDescription,
        compare_tariffs: Vec<&'a dyn Tariff>
    ) -> Self {
        let oi = Installation::from(&original_installation);
        let ob = original_installation.battery
            .map(|desc| (Battery::from(&desc), desc.strategy));
        let ni = Installation::from(&new_installation);
        let nb = new_installation.battery
            .map(|desc| (Battery::from(&desc), desc.strategy));

        Self {
            original_installation: oi,
            original_tariff,
            original_battery: ob,
            new_installation: ni,
            new_battery: nb,
            compare_tariffs
        }
    }

    pub fn compare(
        &mut self,
        records: &ConsumptionRecords,
        start_date: &NaiveDate,
        end_date: &NaiveDate
    ) {
        let mut original = Simulation {
            installation: self.original_installation,
            tariff: self.original_tariff,
            battery: &mut self.original_battery
        };

        let original_bill = original.simulate(records, start_date, end_date);

        let mut new_bills = Vec::<Bill>::new();
        for tariff in self.compare_tariffs.clone() {
            let mut new = Simulation {
                installation: self.new_installation,
                tariff,
                battery: &mut self.new_battery
            };

            new_bills.push(new.simulate(records, start_date, end_date));

            self.new_battery.as_mut().map(|(_, strategy)| strategy.reset());
        }

        println!("original bill:");
        println!("{}", original_bill);
        println!("new bills:") ;
        for bill in new_bills {
            println!("{}", bill);
        }
    }
}

#[derive(Parser)]
struct Cli {
    #[clap(short, long)]
    consumption_records_file: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(PROJECT_DIRS.cache_dir())?;

    let args = Cli::parse();
    let zip_file = File::open(args.consumption_records_file)?;
    let mut archive = ZipArchive::new(zip_file)?;

    let mut file_names = Vec::<String>::new();
    for file_name in archive.file_names() {
        let file_name = file_name?;
        if file_name.ends_with("Horario.csv") {
            file_names.push(String::from(file_name));
        }
    }

    let mut records = ConsumptionRecords::new();
    for file_name in file_names {
        let mut file = archive.by_name(&*file_name)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        records.parse_file(&contents)?;
    }

    let start_date = NaiveDate::from_ymd_opt(2025, 10, 1).unwrap();
    let indexed_tariff = TD20Indexed::new(NotNan::new(0.123)?, NotNan::zero(), start_date)?;
    let pvpc_tariff = PVPC::new(start_date)?;

    let original_tariff = TD20 {
        power_valley: NotNan::new(0.077)?,
        power_peak: NotNan::new(0.077)?,
        energy_valley: NotNan::new(0.087)?,
        energy_plain: NotNan::new(0.147)?,
        energy_peak: NotNan::new(0.197)?,
        energy_export: NotNan::new(0.07)?,
    };

    let mut comparison = Comparison::new(
        InstallationDescription {
            power_peak: NotNan::new(5.75)?,
            power_valley: NotNan::new(5.75)?,
            battery: None,
        },
        &original_tariff,
        InstallationDescription {
            power_peak: NotNan::new(5.75)?,
            power_valley: NotNan::new(10.)?,
            battery: Some(BatteryDescription {
                capacity: 16.try_into()?,
                discharge_range: (0.10.try_into()?, 1.try_into()?),
                strategy: Box::new(SellAtTop::new(NotNan::new(0.25)?)),
            }),
        },
        vec![&indexed_tariff, &pvpc_tariff],
    );

    comparison.compare(&mut records, &start_date, &NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());

    Ok(())
}




