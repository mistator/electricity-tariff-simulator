use crate::distributors::{Cups, Tension};
use crate::types::TariffType;
use anyhow::anyhow;
use chrono::NaiveDate;
use map_macro::hash_map;
use regex::{Regex, RegexBuilder};
use reqwest::Url;
use reqwest::blocking::{Client, ClientBuilder, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

const BASE_URL: &str = "https://zonaprivada.edistribucion.com";

fn bool_from_int<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: u32 = Deserialize::deserialize(deserializer)?;
    Ok(value == 1)
}

#[derive(Debug, Deserialize, Serialize)]
struct AuraContext {
    pub mode: String,
    pub fwuid: String,
    pub app: String,
    pub loaded: HashMap<String, String>,
    #[serde(default)]
    dn: Vec<String>,
    #[serde(default)]
    globals: HashMap<String, String>,
    #[serde(deserialize_with = "bool_from_int")]
    uad: bool,
}

#[derive(Debug, Serialize)]
struct RequestMessage<T: Serialize> {
    actions: Vec<Action<T>>,
}

#[derive(Debug, Serialize)]
struct Action<T: Serialize> {
    id: String,
    descriptor: String,
    #[serde(rename = "callingDescriptor")]
    calling_descriptor: String,
    params: T
}

#[derive(Debug, Serialize)]
struct AuraForm<'a, T: Serialize> {
    id: u32,
    name: &'a str,
    action: &'a str,
    descriptor: &'a str,
    referrer: &'a str,
    url_name_override: Option<&'a str>,
    params: T,
}

pub struct EDistribucionApi {
    username: String,
    password: String,
    client: Client,
    is_logged_in: bool,
    base_url: Url,
    aura_context: String,
    aura_r: u32,
    token: Option<String>,
    identities: HashMap<String, String>,
}

impl EDistribucionApi {
    fn get_context_from_contents(contents: &String) -> anyhow::Result<String> {
        static RE: LazyLock<Regex> = LazyLock::new(||
            RegexBuilder::new(r"var auraConfig = (.+),$")
                .multi_line(true)
                .build()
                .unwrap());

        let (_, [ctx]) = RE.captures(&contents)
            .ok_or(anyhow!("auraConfig variable not found in index response"))?
            .extract();

        let v: Value = serde_json::from_str(ctx)?;
        let json_ctx = v["context"].clone();
        let aura_ctx: AuraContext = serde_json::from_value(json_ctx)?;

        serde_json::to_string(&aura_ctx).map_err(anyhow::Error::from)
    }

    pub fn new(username: &str, password: &str) -> anyhow::Result<Self> {
        let client = ClientBuilder::new()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36")
            .cookie_store(true)
            .build()?;

        let base_url = Url::parse(BASE_URL)?;
        let url = base_url.join("/areaprivada/s/login/?language=es")?;

        let response = client.get(url).send()?;
        let contents = response.text()?;

        let aura_context = Self::get_context_from_contents(&contents)?;

        Ok(Self {
            username: String::from(username),
            password: String::from(password),
            client,
            is_logged_in: false,
            base_url,
            aura_context,
            aura_r: 1,
            token: None,
            identities: HashMap::new(),
        })
    }

    fn post_aura_form_raw<T: Serialize>(&mut self, form: AuraForm<T>) -> anyhow::Result<Response> {
        let request_message = RequestMessage {
            actions: vec![Action {
                id: format!("{};a", form.id),
                descriptor: format!("apex://{}/ACTION${}", form.name, form.action),
                calling_descriptor: format!("markup://c:{}", form.descriptor),
                params: form.params,
            }]
        };

        let request_form = hash_map! {
            "message" => serde_json::to_string(&request_message)?,
            "aura.context" => self.aura_context.clone(),
            "aura.pageURI" => form.referrer.to_string(),
            "aura.token" => self.token.clone().unwrap_or("null".into()),
        };

        let url_form_name = format!("{}.{}", form.url_name_override.unwrap_or(form.name), form.action);
        let url_path = format!("/areaprivada/s/sfsites/aura?r={}&other.{}=1", self.aura_r, url_form_name);
        let url = self.base_url.join(url_path.as_str())?;

        self.aura_r += 1;

        let request = self.client.post(url).form(&request_form);
        Ok(request.send()?.error_for_status()?)
    }

    fn post_aura_form<T: Serialize>(&mut self, form: AuraForm<T>) -> anyhow::Result<Value> {
        let json = self.post_aura_form_raw(form)?
            .json::<Value>()?;

        Ok(json["actions"][0]["returnValue"].clone())
    }

    fn login(&mut self) -> anyhow::Result<()> {
        let response = self.post_aura_form_raw(AuraForm {
            id: 102,
            name: "LightningLoginFormController",
            action: "login",
            descriptor: "WP_LoginForm",
            url_name_override: Some("LightningLoginForm"),
            referrer: "/areaprivada/s/login/?language=es",
            params: hash_map! {
                "username" => self.username.clone(),
                "password" => self.password.clone(),
                "startUrl" => "".into(),
            },
        })?;

        let body = response.text();
        let value: Value = serde_json::de::from_str(body.unwrap().as_str())?;
        let url = value["events"][0]["attributes"]["values"]["url"].as_str().ok_or(anyhow!("frontdoor url not found"))?;

        self.client.get(url).send()?.error_for_status()?;
        self.client.get(
            self.base_url.join(
                "areaprivada/loginflow/loginFlow.apexp?retURL=%2Fareaprivada%2Fapex%2FCommunitiesLanding&sparkID=Disable_users_with_many_login_attempts_Screen_Flow")?)
            .send()?
            .error_for_status()?;
        let response = self.client.get(self.base_url.join("/areaprivada/s/")?)
            .send()?
            .error_for_status()?;

        self.token = Some(response.cookies()
            .find(|cookie| {
                let name = cookie.name();
                let value = cookie.value();
                println!("cookie: {name} => {value}");
                cookie.name().starts_with("__Host-ERIC_PROD")
            })
            .ok_or(anyhow!("token cookie not found"))?
            .value()
            .to_string());
        self.aura_context = Self::get_context_from_contents(&(response.text()?))?;

        let response = self.post_aura_form(AuraForm {
            id: 115,
            name: "WP_Monitor_CTRL",
            action: "getLoginInfo",
            descriptor: "WP_Monitor",
            referrer: "/areaprivada/s/",
            url_name_override: None,
            params: hash_map! {
                "serviceNumber" => ""
            },
        })?;

        let auth_list = response["authList"].as_array().ok_or(anyhow!("auth list not found"))?;
        for value in auth_list {
            let value = value.as_object().ok_or(anyhow!("auth list not object"))?;
            self.identities.insert(
                value["value"].as_str().ok_or(anyhow!(""))?.to_string(),
                value["label"].as_str().ok_or(anyhow!(""))?.to_string(),
            );
        }

        self.is_logged_in = true;

        Ok(())
    }

    pub fn get_cups_list(&mut self) -> anyhow::Result<Vec<Cups>> {
        if !self.is_logged_in {
            self.login()?;
        }

        let mut cups_list = Vec::new();
        let keys = self.identities.keys().cloned().collect::<Vec<_>>();
        for id in keys {
            let response = self.post_aura_form(AuraForm {
                id: 378,
                name: "WP_ConsultaSuministros",
                action: "getAllCUPS",
                referrer: "/areaprivada/s/wpmysupplies",
                descriptor: "WP_MySuppliesForm",
                url_name_override: None,
                params: hash_map! {
                    "visSelected" => id.clone()
                },
            })?;

            cups_list.extend(
                response["lstCups"]
                    .as_array()
                    .ok_or(anyhow!(""))?
                    .iter()
                    .filter_map(|cups| {
                        Some(Cups {
                            id: cups["Id"].as_str()?.into(),
                            address: cups["Direccion"].as_str()?.into(),
                            tension: match cups["TipoTension"].as_str()? {
                                "BT" => Tension::BT,
                                "AT" => Tension::AT,
                                _ => return None,
                            },
                            tariff_type: match cups["ContratoATR_Tarifa"].as_str()? {
                                "2.0TD" => TariffType::TD20,
                                _ => return None,
                            },
                            company: cups["Comercializadora"].as_str()?.into(),
                            owner_id: id.clone(),
                        })
                    })
            );
        }

        Ok(cups_list)
    }

    fn get_latest_file(&mut self, role_id: &String) -> anyhow::Result<Option<String>> {
        let files = self.post_aura_form(AuraForm {
            id: 236,
            name: "WP_Download_Transfer_CTRL",
            action: "getFiles",
            descriptor: "WP_Download_Transfer_Table",
            referrer: "/areaprivada/s/wp-massivemeasuredownload-v3",
            url_name_override: None,
            params: hash_map! {
                "roleId" => Value::String(role_id.clone()),
                "type" => Value::Array(
                    Vec::from_iter(["01", "50", "51"].map(|item| Value::String(item.to_string())))),
            },
        })?;

        let mut files = files["data"]["lstFiles"].as_array().ok_or(anyhow!(""))?.clone();
        files.sort_by_key(|item| item["Title"].as_str().unwrap().to_string().clone());
        files.reverse();

        Ok(files.first().map(|item| item["fileid"].as_str().unwrap().to_string()))
    }

    pub fn get_data_download_url(&mut self, cups: &Cups, start_date: &NaiveDate, end_date: &NaiveDate) -> anyhow::Result<Url> {
        if !self.is_logged_in {
            self.login()?;
        }

        let contract_data = self.post_aura_form(AuraForm {
            id: 2667,
            name: "WP_Measure_v3_CTRL",
            action: "getListCups",
            descriptor: "WP_Massive_Measure_Download_v3",
            referrer: "/areaprivada/s/wp-massivemeasuredownload-v3",
            url_name_override: None,
            params: hash_map! {
                "sIdentificador" => cups.owner_id.clone(),
            },
        })?;

        let contract_data = contract_data["data"]["lstCups"]
            .as_array()
            .ok_or(anyhow!("invalid response"))?
            .iter()
            .filter(|contract| contract["CUPs__r"]["Id"].as_str() == Some(cups.id.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        let cups_ids = contract_data
            .iter()
            .map(|contract| contract["Id"]
                .as_str()
                .map(|item| Value::String(item.to_string()))
                .ok_or(anyhow!("")))
            .collect::<Result<Vec<_>, _>>()?;
        let cups_ids = Value::Array(cups_ids);

        let latest_file = self.get_latest_file(&cups.owner_id)?;

        let params = hash_map! {
            "roleId" => Value::String(cups.owner_id.clone()),
            "lstCupsIds" => cups_ids,
            "data" => Value::Array(contract_data),
            "startDate" => Value::String(start_date.format("%d/%m/%Y").to_string()),
            "endDate" => Value::String(end_date.format("%d/%m/%Y").to_string()),
            "downloadType" => Value::Number(1.into())
        };

        self.post_aura_form(AuraForm {
            id: 2969,
            name: "WP_Measure_v3_CTRL",
            action: "createZip",
            descriptor: "WP_Massive_Measure_Filter2_v3",
            referrer: "/areaprivada/s/wp-massivemeasuredownload-v3",
            url_name_override: None,
            params,
        })?;

        let mut retries = 15;
        while retries > 0 {
            thread::sleep(Duration::from_secs(3));

            let file = self.get_latest_file(&cups.owner_id)?;
            if file != latest_file && let Some(file) = file {
                let url = self.base_url.join(
                    format!("/areaprivada/sfc/servlet.shepherd/version/download/{file}").as_str()
                )?;
                return Ok(url);
            }

            retries -= 1;
        }

        Err(anyhow!("zip file not received"))
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use crate::distributors::edistribucion::EDistribucionApi;

    #[test]
    fn build_api_client() {
        let ed_api = EDistribucionApi::new("", "");
        assert!(ed_api.is_ok());
        let mut ed_api = ed_api.unwrap();
        ed_api.login().unwrap();
        let cups = ed_api.get_cups_list().unwrap();
        let url = ed_api.get_data_download_url(
            cups.first().unwrap(),
            &NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            &NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
        );

        assert!(url.is_ok());
    }
}
