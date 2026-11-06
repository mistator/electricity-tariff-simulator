use crate::utils::DateHour;
use chrono::NaiveDate;
use csv::ReaderBuilder;
use derive_more::{Deref, DerefMut};
use ordered_float::NotNan;
use serde::Deserialize;
use std::error::Error;

#[derive(Debug, Deserialize)]
pub enum TariffType {
    TD20,
}

pub type ConsumptionRecord = (DateHour, NotNan<f64>);

#[derive(Deref, DerefMut)]
pub struct ConsumptionRecords(Vec<ConsumptionRecord>);

impl ConsumptionRecords {
    pub fn new() -> Self { Self { 0: Vec::new() } }

    pub fn parse_file(&mut self, csv_contents: &String) -> Result<(), Box<dyn Error>> {
        let mut rdr = ReaderBuilder::new()
            .delimiter(b';')
            .has_headers(true)
            .from_reader(csv_contents.as_bytes());

        {
            rdr.headers()?;
        }

        for record in rdr.records() {
            let record = record?;

            let date_raw = record.get(1).unwrap().split("/").collect::<Vec<&str>>();
            let day = date_raw[0].parse::<u32>()?;
            let month = date_raw[1].parse::<u32>()?;
            let year = date_raw[2].parse::<i32>()?;

            let hour_raw = record.get(2).unwrap();
            if hour_raw.is_empty() {
                continue;
            }
            let mut hour = hour_raw.parse::<u32>()? - 1;
            if hour == 24 {
                hour = 23
            }


            let import_raw = record.get(3).unwrap();
            if import_raw.is_empty() {
                continue;
            }
            let import = import_raw.replace(",", ".").parse::<f64>()?;

            let export_raw = record.get(4).unwrap();
            if export_raw.is_empty() {
                continue;
            }
            let export = export_raw.replace(",", ".").parse::<f64>()?;

            self.0.push((
                DateHour::new(NaiveDate::from_ymd_opt(year, month, day).unwrap(), hour),
                NotNan::new(import - export)?));
        }

        self.0.sort_by_key(|item| item.0);

        Ok(())
    }

    

    pub fn by_day(&self) -> DayIterator<'_> {
        DayIterator { records: &self, idx: 0 }
    }
}

impl<'a> FromIterator<&'a ConsumptionRecord> for ConsumptionRecords {
    fn from_iter<T: IntoIterator<Item=&'a ConsumptionRecord>>(iter: T) -> Self {
        let mut vec = Vec::new();
        for &item in iter {
            vec.push(item);
        }

        Self(vec)
    }
}


pub struct DayIterator<'a> {
    records: &'a ConsumptionRecords,
    idx: usize,
}

impl<'a> Iterator for DayIterator<'a> {
    type Item = ConsumptionRecords;

    fn next(&mut self) -> Option<Self::Item> {
        let mut chunk = ConsumptionRecords::new();

        let first = self.records.get(self.idx)?;
        chunk.push(first.clone());
        self.idx += 1;

        loop {
            let item = self.records.get(self.idx)?;
            if item.0.date() == first.0.date() {
                chunk.push(*item);
            } else {
                return Some(chunk);
            }
            self.idx += 1
        }

    }
}