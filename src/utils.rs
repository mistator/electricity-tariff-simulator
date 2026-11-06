use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use chrono::Days;
use derive_more::{Deref, IntoIterator};
use directories::ProjectDirs;
use ordered_float::NotNan;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::io::{Read, Seek};
use std::sync::LazyLock;
use zip::ZipArchive;
use zip::read::ZipFile;

#[derive(Serialize, Deserialize, Deref, Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DateHour(#[deref] NaiveDate, u32);

impl DateHour {
    pub fn new(date: NaiveDate, hour: u32) -> Self {
        Self(date, hour)
    }

    pub fn date(&self) -> NaiveDate { self.0 }

    pub fn hour(&self) -> u32 { self.1 }

    pub fn with_hour(&self, hour: u32) -> Self {
        Self(self.0, hour)
    }

    pub fn checked_add_days(&self, days: Days) -> Option<Self> {
        Some(Self(self.0.checked_add_days(days)?, self.1))
    }
}

impl Display for DateHour {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}T{}:00:00", self.date(), self.hour())
    }
}

#[derive(Clone)]
pub struct EnergyPrices {
    pub import: NotNan<f64>,
    pub export: NotNan<f64>,
}

#[derive(Clone, IntoIterator)]
pub struct EnergyPricesSlice {
    #[into_iterator(owned, ref)] pub records: Vec<(u32, EnergyPrices)>,
    import_indices: Vec<usize>,
    export_indices: Vec<usize>,
}

fn arg_sort_by_key<T, K, F>(input: &Vec<T>, key: F) -> Vec<usize>
where
    F: Fn(&T) -> K,
    K: Ord,
{
    let mut indices = Vec::from_iter(0..input.len());
    indices.sort_by_key(|&i| key(&input[i]));
    indices
}

impl EnergyPricesSlice {
    pub fn new(records: Vec<(u32, EnergyPrices)>) -> Self {
        let mut import_indices = arg_sort_by_key(&records, |(_, item)| item.import);
        let mut export_indices = arg_sort_by_key(&records, |(_, item)| item.export);

        import_indices.reverse();
        export_indices.reverse();

        Self {
            records,
            import_indices,
            export_indices,
        }
    }

    pub fn min_import(&self) -> NotNan<f64> {
        self.records[self.min_import_idx()].1.import
    }

    pub fn min_import_hour(&self) -> u32 {
        self.records[self.min_import_idx()].0
    }

    fn min_import_idx(&self) -> usize {
        *self.import_indices.last().unwrap()
    }

    pub fn max_export(&self) -> NotNan<f64> {
        self.records[self.max_export_idx()].1.export
    }

    pub fn max_export_hour(&self) -> u32 {
        self.records[self.max_export_idx()].0
    }

    fn max_export_idx(&self) -> usize {
        *self.export_indices.first().unwrap()
    }

    pub fn spread(&self) -> NotNan<f64> {
        self.max_export() - self.min_import()
    }

    pub fn is_top_n_hours_import(&self, n: u8, hour: u32) -> bool {
        self.import_indices.iter()
            .take(n as usize)
            .any(|&i| self.records[i].0 == hour)
    }

    pub fn top_n_hours_import(&self, n: u8) -> Vec<(u32, EnergyPrices)> {
        self.import_indices.iter()
            .take(n as usize)
            .map(|&i| self.records[i].clone())
            .collect()
    }

    pub fn is_top_n_hours_export(&self, n: u8, hour: u32) -> bool {
        self.export_indices.iter()
            .take(n as usize)
            .any(|&i| self.records[i].0 == hour)
    }

    pub fn top_n_hours_export(&self, n: u8) -> Vec<(u32, EnergyPrices)> {
        self.export_indices.iter()
            .take(n as usize)
            .map(|&i| self.records[i].clone())
            .collect()
    }


    pub fn is_bottom_n_hours_import(&self, n: u8, hour: u32) -> bool {
        self.import_indices.iter()
            .skip(self.import_indices.len() - n as usize)
            .any(|&i| self.records[i].0 == hour)
    }

    pub fn bottom_n_hours_import(&self, n: u8) -> Vec<(u32, EnergyPrices)> {
        self.import_indices.iter()
            .skip(self.import_indices.len() - n as usize)
            .map(|&i| self.records[i].clone())
            .collect()
    }

    pub fn import_price_at(&self, hour: u32) -> NotNan<f64> {
        self.records.iter()
            .find(|(record_hour, _)| *record_hour == hour)
            .unwrap().1.import
    }

    pub fn extended(&self, other: EnergyPricesSlice) -> Self {
        let mut vec = self.records.clone();
        vec.extend(other.records);
        Self::new(vec)
    }
}

impl<'a> FromIterator<&'a(u32, EnergyPrices)> for EnergyPricesSlice {
    fn from_iter<I: IntoIterator<Item = &'a(u32, EnergyPrices)>>(iter: I) -> Self {
        Self::new(Vec::from_iter(iter.into_iter().cloned()))
    }
}

pub fn make_datetime(year: i32, month: u32, day: u32, hour: u32) -> NaiveDateTime {
    NaiveDateTime::new(
        NaiveDate::from_ymd_opt(year, month, day).unwrap(),
        NaiveTime::from_hms_opt(hour, 0, 0).unwrap())
}

pub fn process_zip_archive_files<T: Read + Seek, F: FnMut(ZipFile<T>) -> anyhow::Result<()>> (
    archive: &mut ZipArchive<T>,
    mut handler: F,
) -> anyhow::Result<()> {
    let file_names = archive
        .file_names()
        .map(|name| name.map(|name| name.to_string()))
        .collect::<Result<Vec<_>, _>>()?;

    for file_name in file_names {
        let file = archive.by_name(&*file_name.clone())?;
        handler(file)?;
    }

    Ok(())
}

pub static PROJECT_DIRS: LazyLock<ProjectDirs> =
    LazyLock::new(|| ProjectDirs::from("", "", "calculadora-luz").unwrap());
