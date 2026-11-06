use crate::types::TariffType;
use serde::Deserialize;

pub mod edistribucion;

#[derive(Deserialize, Debug)]
pub enum Tension {
    BT,
    AT
}

pub struct Cups {
    pub id: String,
    pub address: String,
    pub tension: Tension,
    pub tariff_type: TariffType,
    pub company: String,
    owner_id: String,
}