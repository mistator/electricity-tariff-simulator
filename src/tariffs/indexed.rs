use crate::Tariff;
use crate::tariffs::td20::TD20EnergyPeriod;
use crate::utils::{DateHour, PROJECT_DIRS, process_zip_archive_files};
use anyhow::anyhow;
use chrono::{Days, NaiveDate, Utc};
use derive_more::{Deref, DerefMut, IntoIterator};
use num_traits::Zero;
use ordered_float::NotNan;
use rusty_xml::{XPathObject, XmlDoc, XmlXPathContext, default_parse_options, xml_read_memory, xml_xpath_eval};
use serde::{Deserialize, Serialize};
use std::cmp::min;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;
use std::{fs, ops};
use zip::ZipArchive;

const TD20_ENERGY_TRANSPORT_TOLL_PEAK: f64 = 0.033261; // EUR/KWh
const TD20_ENERGY_SYSTEM_TOLL_PEAK: f64 = 0.064292; // EUR/KWh
const TD20_ENERGY_TRANSPORT_TOLL_PLAIN: f64 = 0.016409; // EUR/KWh
const TD20_ENERGY_SYSTEM_TOLL_PLAIN: f64 = 0.012858; // EUR/KWh
const TD20_ENERGY_TRANSPORT_TOLL_VALLEY: f64 = 0.000077; // EUR/KWh
const TD20_ENERGY_SYSTEM_TOLL_VALLEY: f64 = 0.003215; // EUR/KWh

const TD20_ENERGY_TOLLS_PEAK: NotNan<f64> =
    unsafe { NotNan::new_unchecked(TD20_ENERGY_TRANSPORT_TOLL_PEAK + TD20_ENERGY_SYSTEM_TOLL_PEAK) };
const TD20_ENERGY_TOLLS_PLAIN: NotNan<f64> =
    unsafe { NotNan::new_unchecked(TD20_ENERGY_TRANSPORT_TOLL_PLAIN + TD20_ENERGY_SYSTEM_TOLL_PLAIN) };
const TD20_ENERGY_TOLLS_VALLEY: NotNan<f64> =
    unsafe { NotNan::new_unchecked(TD20_ENERGY_TRANSPORT_TOLL_VALLEY + TD20_ENERGY_SYSTEM_TOLL_VALLEY) };

const PEAK_POWER_COST: NotNan<f64> = unsafe { NotNan::new_unchecked(27.704595) }; // EUR/KW/yr
const VALLEY_POWER_COST: NotNan<f64> = unsafe { NotNan::new_unchecked(0.725255) }; // EUR/KW/yr
const MUNICIPAL_TAX_RATE: NotNan<f64> = unsafe { NotNan::new_unchecked(1.015) };
const DAYS_IN_A_YEAR: NotNan<f64> = unsafe { NotNan::new_unchecked(365.) };

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketPrice {
    pub market_price: NotNan<f64>,
    pub adjustment_services: NotNan<f64>,
    pub other_costs: NotNan<f64>,
    pub futures: NotNan<f64>,
    pub deviation_costs: NotNan<f64>,

    pub losses_coeff: NotNan<f64>,
}

impl MarketPrice {
    pub fn production_cost(&self) -> NotNan<f64> {
        self.market_price + self.adjustment_services + self.other_costs
    }

    pub fn production_cost_pvpc(&self) -> NotNan<f64> {
        self.market_price + self.adjustment_services + self.other_costs + self.futures
    }

    pub fn production_price(&self) -> NotNan<f64> {
        self.production_cost() * self.losses_coeff
    }

    pub fn production_price_pvpc(&self) -> NotNan<f64> {
        self.production_cost_pvpc() * self.losses_coeff
    }

    fn export_price(&self) -> NotNan<f64> {
        self.market_price - self.deviation_costs
    }
}

#[derive(Debug, Serialize, Deserialize, Deref, DerefMut, IntoIterator)]
pub struct PriceRecords(#[into_iterator(owned, ref, ref_mut)] HashMap<DateHour, MarketPrice>);

impl PriceRecords {
    fn to_cache(&self, file_path: &Path) {
        || -> anyhow::Result<()> {
            let data = postcard::to_allocvec(&self)?;
            fs::write(file_path, &data.as_slice())?;

            Ok(())
        }().ok();
    }

    fn from_cache(file_path: &Path) -> Self {
        || -> anyhow::Result<Self> {
            let data = fs::read(file_path)?;
            let result = postcard::from_bytes::<PriceRecords>(&data.as_slice())?;

            Ok(result)
        }().unwrap_or_else(|_| {
            fs::remove_file(file_path).ok();
            PriceRecords(Default::default())
        })
    }
}

#[derive(Clone, Deref, DerefMut)]
struct SeriesValues(HashMap<u32, NotNan<f64>>);

impl SeriesValues {
    pub fn new() -> Self {
        Self(HashMap::new())
    }
}

macro_rules! derive_ops {
    ($trt:ident, $f:ident) => {
        impl ops::$trt for SeriesValues {
            type Output = Self;

            fn $f(self, rhs: Self) -> Self::Output {
                let mut values = SeriesValues::new();

                for (key, value) in self.0.into_iter() {
                    let other_value = rhs.0.get(&key).unwrap();
                    values.insert(key, value.$f(other_value));
                }

                values
            }
        }
    };
}

derive_ops!(Add, add);
derive_ops!(Sub, sub);
derive_ops!(Mul, mul);
derive_ops!(Div, div);

fn get_series(doc: &XmlDoc, ctx: &mut XmlXPathContext, series_name: &str) -> anyhow::Result<SeriesValues> {
    let mut values = SeriesValues::new();

    match xml_xpath_eval("//SeriesTemporales", &ctx).map_err(|err| anyhow!(err))? {
        XPathObject::NodeSet(ns) => for ns_element in ns {
            ctx.xml_xpath_set_context_node(ns_element);
            let xpath = format!("./TerminoCosteHorario[@v='{series_name}']");
            let is_series = match xml_xpath_eval(xpath.as_str(), &ctx).map_err(|err| anyhow!(err))? {
                XPathObject::NodeSet(ns) => ns.len() == 1,
                _ => return Err(anyhow!("error")),
            };

            if is_series {
                match xml_xpath_eval("./Periodo//Intervalo", &ctx).map_err(|err| anyhow!(err))? {
                    XPathObject::NodeSet(ns) => for ns_element in ns {
                        let pos = doc.first_child(ns_element).ok_or(anyhow!("asdf"))?;
                        let ctd = doc.last_child(ns_element).ok_or(anyhow!("asdf"))?;

                        let hour = doc.xml_get_prop(pos, "v").ok_or(anyhow!("asdf"))?;
                        let hour = hour.parse::<u32>()? - 1;

                        let ctd = doc.xml_get_prop(ctd, "v").ok_or(anyhow!("asdf"))?;
                        let ctd = ctd.parse::<f64>()?;

                        values.insert(hour, NotNan::try_from(ctd)?);
                    }
                    _ => {}
                }

                return Ok(values);
            }
        }
        _ => return Err(anyhow!("error")),
    };

    Err(anyhow!("series not found"))
}


pub struct TD20Indexed {
    pub daily_fee: NotNan<f64>,
    pub kwh_margin: NotNan<f64>,
    pub price_records: PriceRecords,
}

impl TD20Indexed {
    pub fn process_chunk(price_records: &mut PriceRecords, start_date: NaiveDate, end_date: NaiveDate) -> anyhow::Result<()> {
        let n_days = (end_date - start_date).num_days();

        if start_date.iter_days().take(n_days as usize).all(|item| {
            // We assume if the first hour is in the records, then all are
            price_records.contains_key(&DateHour::new(item, 0))
        }) {
            // All records are cached, return
            return Ok(());
        };

        let start_date_str = start_date
            .checked_sub_days(Days::new(1))
            .unwrap()
            .format("%Y-%m-%d")
            .to_string();
        let end_date_str = end_date.format("%Y-%m-%d").to_string();

        let url = format!("https://api.esios.ree.es/archives/80/download?\
            date_type=publicacion&\
            end_date={end_date_str}T23%3A59%3A59%2B00%3A00&\
            locale=es&\
            start_date={start_date_str}T00%3A00%3A00%2B00%3A00");

        let result = reqwest::blocking::get(url).expect("failed to make request");
        let bytes = result.bytes().expect("failed to get bytes");
        let cursor = Cursor::new(bytes);
        let mut archive = ZipArchive::new(cursor)?;

        process_zip_archive_files(&mut archive, |mut file| {
            let mut buffer = [0u8; 1_024_000]; // 1Mb
            let size = file.read(&mut buffer)?;

            let date_str = file.name()?;
            let date_str = date_str.split('_').last().ok_or(anyhow!("asdf"))?;
            let year = date_str[0..4].parse::<i32>()?;
            let month = date_str[4..6].parse::<u32>()?;
            let day = date_str[6..8].parse::<u32>()?;

            let doc = xml_read_memory(&buffer[0..size], None, None, default_parse_options())?;
            let mut ctx = XmlXPathContext::xml_xpath_new_context(&doc);

            let market_prices = get_series(&doc, &mut ctx, "Pmh")?;
            let adjustment_services = get_series(&doc, &mut ctx, "SAh")?;
            let other_costs = get_series(&doc, &mut ctx, "OCh")?;
            let futures = get_series(&doc, &mut ctx, "Tah")?;
            let deviation_costs = get_series(&doc, &mut ctx, "CDSVh")?;

            let pvpc_costs = market_prices.clone() +
                adjustment_services.clone() +
                other_costs.clone() +
                futures.clone();
            let pvpc_prices = get_series(&doc, &mut ctx, "TCUh")?;

            let losses_coeffs = pvpc_prices / pvpc_costs;

            for key in market_prices.keys() {
                price_records.insert(
                    DateHour::new(NaiveDate::from_ymd_opt(year, month, day).unwrap(), *key),
                    MarketPrice {
                        market_price: *market_prices.get(key).ok_or(anyhow!("asdf"))?,
                        adjustment_services: *adjustment_services.get(key).ok_or(anyhow!("asdf"))?,
                        other_costs: *other_costs.get(key).ok_or(anyhow!("asdf"))?,
                        futures: *futures.get(key).ok_or(anyhow!("asdf"))?,
                        deviation_costs: *deviation_costs.get(key).ok_or(anyhow!("asdf"))?,
                        losses_coeff: *losses_coeffs.get(key).ok_or(anyhow!("asdf"))?,
                    });

            }

            Ok(())
        })?;

        Ok(())
    }

    pub fn new(daily_fee: NotNan<f64>, kwh_margin: NotNan<f64>, mut start_date: NaiveDate) -> anyhow::Result<Self> {
        let cache_file = PROJECT_DIRS.cache_dir().join(&Path::new("ree_prices"));
        let cache_file = cache_file.as_path();

        let mut price_records = PriceRecords::from_cache(cache_file);
        let now = Utc::now().date_naive();

        loop {
            let end_date = min(start_date.checked_add_days(Days::new(30)).unwrap(), now);
            Self::process_chunk(&mut price_records, start_date, end_date)?;

            if end_date >= now {
                break;
            }

            start_date = end_date.checked_add_days(Days::new(1)).unwrap();
        }

        price_records.to_cache(cache_file);

        Ok(Self {
            daily_fee,
            kwh_margin,
            price_records,
        })
    }
}

impl Tariff for TD20Indexed {
    fn get_import_rate(&self, datetime: &DateHour) -> NotNan<f64> {
        let record = self.price_records.get(datetime).unwrap();

        let tolls = match TD20EnergyPeriod::from_datetime(datetime) {
            TD20EnergyPeriod::Peak => TD20_ENERGY_TOLLS_PEAK,
            TD20EnergyPeriod::Plain => TD20_ENERGY_TOLLS_PLAIN,
            TD20EnergyPeriod::Valley => TD20_ENERGY_TOLLS_VALLEY,
        };

        MUNICIPAL_TAX_RATE * (record.production_price() + self.kwh_margin) + tolls
    }

    fn get_export_rate(&self, datetime: &DateHour) -> NotNan<f64> {
        let record = self.price_records.get(datetime).unwrap();
        MUNICIPAL_TAX_RATE * record.export_price()
    }

    fn get_daily_power_rate_peak(&self) -> NotNan<f64> {
        PEAK_POWER_COST / DAYS_IN_A_YEAR
    }

    fn get_daily_power_rate_valley(&self) -> NotNan<f64> {
        VALLEY_POWER_COST / DAYS_IN_A_YEAR
    }

    fn get_other_concepts_cost(&self, days: u32) -> NotNan<f64> {
        MUNICIPAL_TAX_RATE * self.daily_fee * NotNan::from(days)
    }
}

pub struct PVPC(TD20Indexed);

impl PVPC {
    pub fn new(start_date: NaiveDate) -> anyhow::Result<Self> {
        let indexed = TD20Indexed::new(NotNan::zero(), NotNan::zero(), start_date)?;
        Ok(Self(indexed))
    }
}

impl Tariff for PVPC {
    fn get_import_rate(&self, datetime: &DateHour) -> NotNan<f64> {
        let record = self.0.price_records.get(datetime).unwrap();

        let tolls = match TD20EnergyPeriod::from_datetime(datetime) {
            TD20EnergyPeriod::Peak => TD20_ENERGY_TOLLS_PEAK,
            TD20EnergyPeriod::Plain => TD20_ENERGY_TOLLS_PLAIN,
            TD20EnergyPeriod::Valley => TD20_ENERGY_TOLLS_VALLEY,
        };

        MUNICIPAL_TAX_RATE * record.production_price_pvpc() + tolls
    }

    fn get_export_rate(&self, datetime: &DateHour) -> NotNan<f64> {
        self.0.get_export_rate(datetime)
    }

    fn get_daily_power_rate_peak(&self) -> NotNan<f64> {
        self.0.get_daily_power_rate_peak()
    }

    fn get_daily_power_rate_valley(&self) -> NotNan<f64> {
        self.0.get_daily_power_rate_valley()
    }
}


#[cfg(test)]
mod tests {
    use crate::tariffs::indexed::TD20Indexed;
    use chrono::NaiveDate;
    use num_traits::Zero;
    use ordered_float::NotNan;

    #[test]
    fn test_omie() {
        let idx = TD20Indexed::new(
            NotNan::new(0.123).unwrap(), NotNan::zero(), NaiveDate::from_ymd_opt(2026, 10, 8).unwrap());
        for (key, value) in idx.unwrap().price_records {
            println!("{:?}: {:?}", key, value.production_price_pvpc());
        }
    }
}