use std::collections::HashSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use aes::Aes256;
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use cbc::Encryptor;
use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use clap::Parser;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::Rng;
use reqwest::Method;
use reqwest::blocking::{Client, Response};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, ORIGIN, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

const SECRET: &str = "8c1ef35c1a24f94ce6422f3c4b77e19bec2aaec9c0d72251b82ccf40b22561a84c876c19d2cb9a";
const ENDPOINT: &str = "https://mrr.readoor.cn/api/3.1/stat/v1/b/stat31/stat/pStatIf";
const SECTIONS_ENDPOINT: &str = "https://api3.readoor.cn/api/3.0/app/v1/dms/spu/sections";
const APP_INFO_ENDPOINT: &str = "https://api3.readoor.cn/api/3.1/app/v1/app/info";
const IAAA_LOGIN_ENDPOINT: &str = "https://iaaa.pku.edu.cn/iaaa/oauthlogin.do";
const NEW_KEY: &str = "AUe#2jE31o90";
const REFERER_VALUE: &str = "https://byyxt.pupedu.cn/550278742975483904/c/pc/viewer?spu_guid=570535132977475584&group_id=16211&training_id=17296&project_id=408520&section_guid=570542196487401472";
const COURSE_CLASS_ID: &str = "16211";
const COURSE_TRAIN_ID: &str = "17296";
const COURSE_PROJECT_ID: &str = "408520";
const COURSE_TASK_GUID: &str = "588410513113788416";
const COURSE_SPU_GUID: &str = "570535132977475584";
const COURSE_SPU_TYPE: i64 = 302;
const DEFAULT_SECTION_GUID: &str = "570542196487401472";
const DEFAULT_COURSEWARE_TYPE: i64 = 104;
const DEFAULT_SECTION_TYPE: i64 = 403;
const DEFAULT_MEDIA_DURATION: f64 = 1009.36;
const DEFAULT_STUDY_TIME: i64 = 40;
const DEFAULT_SEQUENCE_ID: i64 = 7;
const DEFAULT_PLATFORM_CODE: &str = "pweb";

#[derive(Parser, Debug)]
#[command(
    name = "auto_learn",
    about = "Probe Readoor pStatIf, with optional PKU IAAA login and token_code exchange."
)]
struct Args {
    #[arg(long, env = "READOOR_TOKEN")]
    token: Option<String>,
    #[arg(long, env = "READOOR_USERNAME")]
    username: Option<String>,
    #[arg(long, env = "READOOR_PASSWORD")]
    password: Option<String>,
    #[arg(long = "token-code")]
    token_code: Option<String>,
    #[arg(long = "iaaa-oauth-url")]
    iaaa_oauth_url: Option<String>,
    #[arg(long = "app-guid", default_value = "550278742975483904")]
    app_guid: String,
    #[arg(long = "terminal-id", default_value = "4")]
    terminal_id: String,
    #[arg(long = "login-only")]
    login_only: bool,
    #[arg(long = "spu-guid", default_value = COURSE_SPU_GUID)]
    spu_guid: String,
    #[arg(long = "module-id")]
    module_id: Option<String>,
    #[arg(long = "section-guid")]
    section_guid: Option<String>,
    #[arg(long = "task-guid", default_value = COURSE_TASK_GUID)]
    task_guid: String,
    #[arg(long = "sections-file")]
    sections_file: Option<String>,
    #[arg(long = "list-sections")]
    list_sections: bool,
    #[arg(long = "choose-section")]
    choose_section: bool,
    #[arg(long = "session-id")]
    session_id: Option<String>,
    #[arg(long = "sequence-id")]
    sequence_id: Option<i64>,
    #[arg(long = "study-time", default_value_t = DEFAULT_STUDY_TIME)]
    study_time: i64,
    #[arg(long = "position")]
    position: Option<f64>,
    #[arg(long = "incomplete")]
    incomplete: bool,
    #[arg(long = "dump-only")]
    dump_only: bool,
    #[arg(long = "debug")]
    debug: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct AppInfoResponse {
    status: Value,
    data: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct LoginResponse {
    success: bool,
    token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TokenExchangeResponse {
    status: Value,
    data: Value,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let client = Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("failed to build HTTP client")?;

    let app_info = fetch_app_info(&client, &args.app_guid, &args.terminal_id)?;
    let mut token = args.token.clone();
    let mut token_code = args.token_code.clone();

    if token.is_none() && token_code.is_none() {
        if let (Some(username), Some(password)) = (&args.username, &args.password) {
            let (new_token_code, login_meta) = login_iaaa_for_token_code(
                &client,
                username,
                password,
                args.iaaa_oauth_url.as_deref(),
                Some(&app_info),
            )?;
            if args.debug {
                println!("=== IAAA callback ===");
                println!("{}", serde_json::to_string_pretty(&login_meta)?);
            }
            token_code = Some(new_token_code);
        }
    }

    if token.is_none() {
        if let Some(token_code_value) = token_code.as_deref() {
            let token_payload = exchange_token_code(&client, token_code_value, &app_info)?;
            if args.debug {
                println!("=== token exchange ===");
                println!("{}", serde_json::to_string_pretty(&token_payload)?);
            }
            token = token_payload
                .get("data")
                .and_then(|v| v.get("token"))
                .and_then(|v| v.get("token"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
    }

    if args.login_only {
        let token = token.context("Login did not produce a Bearer token.")?;
        println!("\n=== bearer token ===");
        println!("{token}");
        return Ok(());
    }

    let mut sections_payload: Option<Value> = None;
    if let Some(path) = &args.sections_file {
        sections_payload = Some(load_sections_file(path)?);
    } else if args.list_sections || args.module_id.is_some() || args.section_guid.is_some() {
        let token_value = token
            .as_deref()
            .context("Missing token for sections API. Pass --token or login with --username/--password.")?;
        let module_id = args
            .module_id
            .as_deref()
            .context("module_id is required when fetching sections from the API.")?;
        let sections_response = fetch_sections(&client, token_value, &args.spu_guid, module_id)?;
        let status_code = sections_response.status();
        let text = sections_response.text().context("failed to read sections response body")?;
        if args.debug {
            println!("=== sections HTTP response ===");
            println!("status_code={status_code}");
            println!("{text}");
        }
        sections_payload = Some(serde_json::from_str(&text).context("failed to parse sections response JSON")?);
    }

    if args.list_sections {
        if let Some(payload) = &sections_payload {
            println!("\n=== sections summary ===");
            for section in sections(payload)? {
                println!("{}", serde_json::to_string(&format_section_summary(section))?);
            }
            if args.dump_only {
                return Ok(());
            }
        }
    }

    let selected_sections = if let Some(payload) = &sections_payload {
        if args.choose_section && args.section_guid.is_none() {
            prompt_for_sections(payload)?
        } else {
            choose_sections(payload, args.section_guid.as_deref())?
        }
    } else {
        vec![None]
    };

    for (index, section) in selected_sections.iter().enumerate() {
        let mut payload = build_payload(
            &args.spu_guid,
            &args.task_guid,
            !args.incomplete,
            args.session_id.as_deref(),
            args.sequence_id,
            Some(args.study_time),
            args.position,
        );
        apply_app_info_to_payload(&mut payload, &app_info)?;

        if let Some(section_value) = section {
            if args.debug {
                println!(
                    "\n=== chosen section {}/{} ===",
                    index + 1,
                    selected_sections.len()
                );
                println!(
                    "{}",
                    serde_json::to_string_pretty(&format_section_summary(section_value))?
                );
            }
            apply_section_to_payload(
                &mut payload,
                section_value,
                Some(&args.spu_guid),
                Some(&args.task_guid),
            )?;
        }

        let encrypted = encrypt_payload(&payload)?;

        if args.debug || args.dump_only {
            println!(
                "\n=== JSON payload {}/{} ===",
                index + 1,
                selected_sections.len()
            );
            println!("{}", serde_json::to_string_pretty(&payload)?);
            println!(
                "\n=== Form payload {}/{} ===",
                index + 1,
                selected_sections.len()
            );
            println!("{}", serde_json::to_string_pretty(&encrypted)?);
        }

        if args.dump_only {
            continue;
        }

        let token_value = token
            .as_deref()
            .context("Missing token. Pass --token or login with --username/--password.")?;
        let response = send_probe(&client, token_value, &encrypted)?;
        let status_code = response.status();
        let text = response.text().context("failed to read probe response body")?;
        if args.debug {
            println!(
                "\n=== HTTP response {}/{} ===",
                index + 1,
                selected_sections.len()
            );
            println!("status_code={status_code}");
            println!("{text}");
        } else {
            let summary = summarize_response(status_code.as_u16(), &text);
            println!("{summary}");
        }
    }

    Ok(())
}

fn sha256_key() -> [u8; 32] {
    let digest = Sha256::digest(SECRET.as_bytes());
    let mut key = [0_u8; 32];
    key.copy_from_slice(&digest);
    key
}

fn now_timestamp_ms() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?;
    Ok(duration.as_millis() as i64)
}

fn md5_hex(value: &str) -> String {
    format!("{:x}", md5::compute(value.as_bytes()))
}

fn build_signed_form(extra: &[(&str, String)], ts: Option<String>, nonce: Option<String>) -> Vec<(String, String)> {
    let ts = ts.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_else(|_| "0".to_string())
    });
    let nonce = nonce.unwrap_or_else(|| Uuid::new_v4().to_string());
    let sign_seed = format!("{NEW_KEY}{ts}{nonce}");
    let sign = md5_hex(&format!("{}{}{}", md5_hex(&sign_seed), ts, nonce));

    let mut form = vec![
        ("ts".to_string(), ts),
        ("nonce".to_string(), nonce),
        ("sign".to_string(), sign),
        ("v".to_string(), "1.0.0".to_string()),
    ];
    for (key, value) in extra {
        form.push(((*key).to_string(), value.clone()));
    }
    form
}

fn build_sections_form(spu_guid: &str, module_id: &str) -> Vec<(String, String)> {
    build_signed_form(
        &[
            ("spu_guid", spu_guid.to_string()),
            ("module_id", module_id.to_string()),
        ],
        None,
        None,
    )
}

fn build_app_info_form(app_guid: &str, terminal_id: &str) -> Vec<(String, String)> {
    build_signed_form(
        &[
            ("app_guid", app_guid.to_string()),
            ("terminal_id", terminal_id.to_string()),
        ],
        None,
        None,
    )
}

fn build_other_token_form(
    token_code: &str,
    app_guid: &str,
    company_guid: &str,
    idaas_id: &str,
) -> Vec<(String, String)> {
    build_signed_form(
        &[
            ("token_code", token_code.to_string()),
            ("app_guid", app_guid.to_string()),
            ("company_guid", company_guid.to_string()),
            ("idaas_id", idaas_id.to_string()),
        ],
        None,
        None,
    )
}

fn parse_iaaa_oauth_url(oauth_url: &str) -> Result<(String, String)> {
    let parsed = Url::parse(oauth_url).context("invalid IAAA oauth URL")?;
    let mut redir_url = None;
    let mut app_id = None;
    for (key, value) in parsed.query_pairs() {
        if key == "redirectUrl" {
            redir_url = Some(value.into_owned());
        } else if key == "appID" {
            app_id = Some(value.into_owned());
        }
    }

    match (redir_url, app_id) {
        (Some(redir_url), Some(app_id)) => Ok((redir_url, app_id)),
        _ => bail!("IAAA oauth URL must contain redirectUrl and appID query params"),
    }
}

fn build_callback_url(app_guid: &str, path: Option<&str>, extra: Option<&[(&str, &str)]>) -> String {
    let cb = Uuid::new_v4().simple().to_string()[..16].to_string();
    let path = path
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("/{app_guid}/home"));
    let mut query = vec![("logintype".to_string(), "sf".to_string()), ("cb".to_string(), cb)];
    if let Some(extra_pairs) = extra {
        for (key, value) in extra_pairs {
            query.push(((*key).to_string(), (*value).to_string()));
        }
    }
    let qs = query
        .into_iter()
        .map(|(key, value)| {
            format!(
                "{key}={}",
                utf8_percent_encode(&value, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("https://byyxt.pupedu.cn{path}?{qs}")
}

fn build_beida_entry_url(app_info: &Value, callback_url: Option<String>, a_uri: Option<Value>) -> Result<String> {
    let callback_url = callback_url.unwrap_or_else(|| {
        build_callback_url(
            app_info
                .get("app_guid")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            None,
            Some(&[("f", "bd"), ("r", "2")]),
        )
    });
    let a_uri = a_uri.unwrap_or_else(|| json!({}));
    let base = app_info
        .get("idp")
        .and_then(|v| v.get("domain"))
        .and_then(Value::as_str)
        .unwrap_or("https://idp.readoor.cn/")
        .trim_end_matches('/');

    let app_guid = app_info_field_string(app_info, &["app_guid"])?;
    let idaas_app_id = app_info_field_string(app_info, &["idp", "idaas_app_id"])?;
    let company_guid = app_info_field_string(app_info, &["company_guid"])?;
    let a_uri_text = serde_json::to_string(&a_uri)?;

    Ok(format!(
        "{base}/api/3.0/idp/v1/ag/bd?&terminal_id=4&mode=20&app_guid={}&appid={}&company_guid={}&callback={}&a_uri={}",
        utf8_percent_encode(&app_guid, NON_ALPHANUMERIC),
        utf8_percent_encode(&idaas_app_id, NON_ALPHANUMERIC),
        utf8_percent_encode(&company_guid, NON_ALPHANUMERIC),
        utf8_percent_encode(&callback_url, NON_ALPHANUMERIC),
        utf8_percent_encode(&a_uri_text, NON_ALPHANUMERIC),
    ))
}

fn fetch_dynamic_iaaa_oauth_url(client: &Client, app_info: &Value) -> Result<String> {
    let mut a_uri = Map::new();
    if let Some(modes) = app_info
        .get("idp")
        .and_then(|v| v.get("config"))
        .and_then(|v| v.get("mode_config"))
        .and_then(Value::as_array)
    {
        for mode in modes {
            if mode.get("mode_id").map(value_to_string).transpose()?.as_deref() == Some("20") {
                if let Some(rel) = mode
                    .get("ext_config")
                    .and_then(|v| v.get("rel"))
                    .and_then(Value::as_object)
                {
                    for key in ["enterprise_id", "org_id"] {
                        if let Some(value) = rel.get(key) {
                            if !value.is_null() {
                                let text = value_to_string(value)?;
                                if !text.is_empty() {
                                    a_uri.insert(key.to_string(), Value::String(text));
                                }
                            }
                        }
                    }
                }
                break;
            }
        }
    }

    let entry_url = build_beida_entry_url(client_app_info_with_guid(app_info)?, None, Some(Value::Object(a_uri)))?;
    let response = client
        .get(&entry_url)
        .send()
        .context("failed to fetch dynamic IAAA oauth URL")?;
    let status = response.status();
    let location = header_string(response.headers(), "location");
    if location
        .as_deref()
        .filter(|value| value.contains("iaaa.pku.edu.cn/iaaa/oauth.jsp"))
        .is_none()
    {
        bail!(
            "Could not fetch dynamic IAAA oauth URL from beida entrypoint. status={} location={:?}",
            status,
            location
        );
    }
    location.context("missing Location header")
}

fn fetch_app_info(client: &Client, app_guid: &str, terminal_id: &str) -> Result<Value> {
    let response = client
        .post(APP_INFO_ENDPOINT)
        .form(&build_app_info_form(app_guid, terminal_id))
        .send()
        .context("app info request failed")?;
    let payload: AppInfoResponse = response.json().context("failed to decode app info response")?;
    if value_to_string(&payload.status)? != "1" {
        bail!("app info request failed: {}", serde_json::to_string(&json!({"status": payload.status, "data": payload.data}))?);
    }
    let mut app_info = payload.data;
    if let Some(obj) = app_info.as_object_mut() {
        obj.insert("app_guid".to_string(), Value::String(app_guid.to_string()));
    }
    Ok(app_info)
}

fn login_iaaa_for_token_code(
    client: &Client,
    username: &str,
    password: &str,
    oauth_url: Option<&str>,
    app_info: Option<&Value>,
) -> Result<(String, Value)> {
    let oauth_url = match oauth_url {
        Some(value) => value.to_string(),
        None => {
            let app_info = app_info.context("Need either oauth_url or app_info to start IAAA login")?;
            fetch_dynamic_iaaa_oauth_url(client, app_info)?
        }
    };

    let (redir_url, app_id) = parse_iaaa_oauth_url(&oauth_url)?;

    // Mirror a browser/session flow more closely: open oauth.jsp first so the
    // IAAA domain can set any required session cookies before oauthlogin.do.
    client
        .get(&oauth_url)
        .header(
            USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36",
        )
        .send()
        .context("failed to open IAAA oauth.jsp before login")?;

    let response = client
        .post(IAAA_LOGIN_ENDPOINT)
        .header(
            ORIGIN,
            HeaderValue::from_static("https://iaaa.pku.edu.cn"),
        )
        .header(
            REFERER,
            HeaderValue::from_str(&oauth_url).context("invalid oauth_url for Referer header")?,
        )
        .header(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36",
            ),
        )
        .form(&[
            ("appid", app_id.as_str()),
            ("userName", username),
            ("password", password),
            ("randCode", ""),
            ("smsCode", ""),
            ("otpCode", ""),
            ("redirUrl", redir_url.as_str()),
        ])
        .send()
        .context("IAAA login request failed")?;
    let login_payload: LoginResponse = response.json().context("failed to decode IAAA login response")?;
    if !login_payload.success {
        bail!(
            "IAAA login failed: {}",
            serde_json::to_string(&json!({"success": login_payload.success, "token": login_payload.token}))?
        );
    }
    let oauth_token = login_payload
        .token
        .context("IAAA login did not return token")?;

    let rand_value: f64 = rand::thread_rng().gen_range(0.0..1.0);
    let callback_response = client
        .request(Method::GET, &redir_url)
        .query(&[
            ("_rand", rand_value.to_string()),
            ("token", oauth_token.clone()),
        ])
        .send()
        .context("IAAA callback request failed")?;

    let status = callback_response.status();
    let location = header_string(callback_response.headers(), "location")
        .unwrap_or_else(|| callback_response.url().to_string());
    let location_url = Url::parse(&location).context("invalid callback redirect URL")?;
    let token_code = location_url
        .query_pairs()
        .find(|(key, _)| key == "token_code")
        .map(|(_, value)| value.into_owned())
        .context(format!(
            "Could not extract token_code from IAAA callback. status={} location={location:?}",
            status
        ))?;

    Ok((
        token_code,
        json!({
            "oauth_token": oauth_token,
            "oauth_url": oauth_url,
            "location": location,
        }),
    ))
}

fn exchange_token_code(client: &Client, token_code: &str, app_info: &Value) -> Result<Value> {
    let endpoint = format!(
        "{}/api/3.0/idp/v1/s/ag/token",
        app_info_field_string(app_info, &["domain", "idp"])?.trim_end_matches('/')
    );
    let form_payload = build_other_token_form(
        token_code,
        &app_info_field_string(app_info, &["app_guid"])?,
        &app_info_field_string(app_info, &["company_guid"])?,
        &app_info_field_string(app_info, &["idp", "idaas_id"])?,
    );
    let response = client
        .post(endpoint)
        .form(&form_payload)
        .send()
        .context("token_code exchange request failed")?;
    let payload: TokenExchangeResponse = response
        .json()
        .context("failed to decode token_code exchange response")?;
    if value_to_string(&payload.status)? != "1" {
        bail!(
            "token_code exchange failed: {}",
            serde_json::to_string(&json!({"status": payload.status, "data": payload.data}))?
        );
    }
    Ok(json!({
        "status": payload.status,
        "data": payload.data,
    }))
}

fn dynamic_done_field(timestamp_ms: i64) -> String {
    let encoded = BASE64_STANDARD.encode(timestamp_ms.to_string().as_bytes());
    encoded.chars().skip(2).take(6).collect()
}

fn build_payload(
    spu_guid: &str,
    task_guid: &str,
    complete: bool,
    session_id: Option<&str>,
    sequence_id: Option<i64>,
    study_time: Option<i64>,
    position: Option<f64>,
) -> Value {
    let now_ms = now_timestamp_ms().unwrap_or_default();
    let now_s = now_ms / 1000;
    let lesson_study_time = study_time.unwrap_or(DEFAULT_STUDY_TIME);
    let lesson_position = position.unwrap_or(DEFAULT_MEDIA_DURATION);
    let lesson_session_id = session_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let lesson_sequence_id = sequence_id.unwrap_or(DEFAULT_SEQUENCE_ID);

    let mut lesson = Map::new();
    lesson.insert("media_theory_length".to_string(), json!(DEFAULT_MEDIA_DURATION));
    lesson.insert("max_position".to_string(), json!(lesson_position));
    lesson.insert("position".to_string(), json!(lesson_position));
    lesson.insert("study_time".to_string(), json!(lesson_study_time));
    lesson.insert("end_time".to_string(), json!(now_s));
    lesson.insert("start_time".to_string(), json!(now_s - lesson_study_time));
    lesson.insert("session_id".to_string(), json!(lesson_session_id));
    lesson.insert("sequence_id".to_string(), json!(lesson_sequence_id));
    lesson.insert("section_guid".to_string(), json!(DEFAULT_SECTION_GUID));
    lesson.insert("courseware_type".to_string(), json!(DEFAULT_COURSEWARE_TYPE));
    lesson.insert("section_type".to_string(), json!(DEFAULT_SECTION_TYPE));
    lesson.insert("task_guid".to_string(), json!(task_guid));

    let done_key = dynamic_done_field(now_ms);
    lesson.insert(
        done_key,
        if complete { Value::String("1".to_string()) } else { json!(0) },
    );

    if complete {
        lesson.insert("position".to_string(), json!(DEFAULT_MEDIA_DURATION));
        lesson.insert("max_position".to_string(), json!(DEFAULT_MEDIA_DURATION));
    }

    json!({
        "base_data": {
            "app_id": 0,
            "company_id": 0,
            "time_stamp": now_ms,
            "class_id": COURSE_CLASS_ID,
            "train_id": COURSE_TRAIN_ID,
            "project_id": COURSE_PROJECT_ID,
            "platform_code": DEFAULT_PLATFORM_CODE,
            "item_id": spu_guid,
            "spu_type": COURSE_SPU_TYPE,
        },
        "lesson_data": [Value::Object(lesson)],
    })
}

fn apply_section_to_payload(
    payload: &mut Value,
    section: &Value,
    spu_guid: Option<&str>,
    task_guid: Option<&str>,
) -> Result<()> {
    if let Some(spu_guid) = spu_guid {
        payload["base_data"]["item_id"] = json!(spu_guid);
    }
    let lesson = payload["lesson_data"]
        .get_mut(0)
        .context("payload.lesson_data[0] missing")?;
    lesson["section_guid"] = json!(section_field_string(section, "section_guid")?);
    lesson["courseware_type"] = json!(section_field_i64(section, "courseware_type")?);
    lesson["section_type"] = json!(section_field_i64(section, "section_type")?);
    if let Some(file_duration) = section.get("file_duration").and_then(Value::as_f64) {
        lesson["media_theory_length"] = json!(file_duration);
        lesson["position"] = json!(file_duration);
        lesson["max_position"] = json!(file_duration);
    }
    if let Some(task_guid) = task_guid {
        lesson["task_guid"] = json!(task_guid);
    }
    Ok(())
}

fn apply_app_info_to_payload(payload: &mut Value, app_info: &Value) -> Result<()> {
    payload["base_data"]["app_id"] = json!(app_info_field_i64(app_info, &["app_id"])?);
    payload["base_data"]["company_id"] = json!(app_info_field_i64(app_info, &["company_id"])?);
    Ok(())
}

fn encrypt_payload(payload: &Value) -> Result<Value> {
    let plain = serde_json::to_vec(payload).context("failed to serialize payload")?;
    let key = sha256_key();
    let iv: [u8; 16] = rand::thread_rng().r#gen();
    let cipher = Encryptor::<Aes256>::new(&key.into(), &iv.into());
    let encrypted = cipher.encrypt_padded_vec_mut::<Pkcs7>(&plain);
    Ok(json!({
        "data": BASE64_STANDARD.encode(encrypted),
        "jfug": BASE64_STANDARD.encode(iv),
    }))
}

fn build_headers(token: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).context("invalid Authorization header")?,
    );
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers.insert(ORIGIN, HeaderValue::from_static("https://byyxt.pupedu.cn"));
    headers.insert(REFERER, HeaderValue::from_static(REFERER_VALUE));
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36",
        ),
    );
    headers.insert("X-Requested-With", HeaderValue::from_static("XMLHttpRequest"));
    Ok(headers)
}

fn send_probe(client: &Client, token: &str, form_payload: &Value) -> Result<Response> {
    let headers = build_headers(token)?;
    let form = json_value_to_string_pairs(form_payload)?;
    client
        .post(ENDPOINT)
        .headers(headers)
        .form(&form)
        .send()
        .context("probe request failed")
}

fn fetch_sections(client: &Client, token: &str, spu_guid: &str, module_id: &str) -> Result<Response> {
    let headers = build_headers(token)?;
    let form_payload = build_sections_form(spu_guid, module_id);
    client
        .post(SECTIONS_ENDPOINT)
        .headers(headers)
        .form(&form_payload)
        .send()
        .context("sections request failed")
}

fn load_sections_file(path: &str) -> Result<Value> {
    let text = fs::read_to_string(path).with_context(|| format!("failed to read sections file: {path}"))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse sections JSON file: {path}"))
}

fn format_section_summary(section: &Value) -> Value {
    json!({
        "section_guid": section.get("section_guid"),
        "section_name": section.get("section_name"),
        "courseware_type": section.get("courseware_type"),
        "section_type": section.get("section_type"),
        "file_duration": section.get("file_duration"),
        "id": section.get("id"),
        "pid": section.get("pid"),
    })
}

fn sections(payload: &Value) -> Result<&Vec<Value>> {
    payload
        .get("data")
        .and_then(|v| v.get("sections"))
        .and_then(Value::as_array)
        .context("No sections found in payload")
}

fn list_playable_sections(payload: &Value) -> Result<Vec<&Value>> {
    let items = sections(payload)?;
    let playable = items
        .iter()
        .filter(|section| {
            section.get("section_type").and_then(Value::as_i64) == Some(403)
                && section.get("courseware_type").is_some_and(|value| !value.is_null())
        })
        .collect::<Vec<_>>();
    if playable.is_empty() {
        bail!("No playable sections found in payload");
    }
    Ok(playable)
}

fn parse_multi_value(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn resolve_sections(payload: &Value, selectors: Option<&[String]>) -> Result<Vec<Option<Value>>> {
    let playable_sections = list_playable_sections(payload)?;
    if selectors.is_none_or(|items| items.is_empty()) {
        return Ok(vec![Some(playable_sections[0].clone())]);
    }

    let mut seen = HashSet::new();
    let mut chosen = Vec::new();
    for selector in selectors.unwrap_or(&[]) {
        let section = if selector.chars().all(|c| c.is_ascii_digit()) {
            let index = selector.parse::<usize>().unwrap_or_default();
            if (1..=playable_sections.len()).contains(&index) {
                Some(playable_sections[index - 1])
            } else {
                None
            }
        } else {
            playable_sections
                .iter()
                .copied()
                .find(|candidate| candidate.get("section_guid").and_then(Value::as_str) == Some(selector.as_str()))
        };

        let section = section.context(format!("Invalid section selector: {selector}"))?;
        let section_guid = section_field_string(section, "section_guid")?;
        if seen.insert(section_guid) {
            chosen.push(Some(section.clone()));
        }
    }
    Ok(chosen)
}

fn prompt_for_sections(payload: &Value) -> Result<Vec<Option<Value>>> {
    let playable_sections = list_playable_sections(payload)?;
    println!("\n=== selectable sections ===");
    for (index, section) in playable_sections.iter().enumerate() {
        let summary = format_section_summary(section);
        println!(
            "[{}] {} | guid={} | type={} | duration={}",
            index + 1,
            summary.get("section_name").and_then(Value::as_str).unwrap_or(""),
            summary
                .get("section_guid")
                .and_then(Value::as_str)
                .unwrap_or(""),
            summary
                .get("courseware_type")
                .filter(|value| !value.is_null())
                .map(value_to_string)
                .transpose()?
                .unwrap_or_default(),
            summary
                .get("file_duration")
                .filter(|value| !value.is_null())
                .map(value_to_string)
                .transpose()?
                .unwrap_or_default()
        );
    }

    loop {
        print!("Choose section number(s) or section_guid(s), comma-separated: ");
        use std::io::Write;
        std::io::stdout().flush().context("failed to flush stdout")?;
        let mut raw = String::new();
        std::io::stdin()
            .read_line(&mut raw)
            .context("failed to read section input")?;
        let raw = raw.trim();
        if raw.is_empty() {
            println!("Please enter a value.");
            continue;
        }
        match resolve_sections(payload, Some(&parse_multi_value(raw))) {
            Ok(sections) => return Ok(sections),
            Err(_) => println!("Invalid selection, try again."),
        }
    }
}

fn choose_sections(payload: &Value, section_guid: Option<&str>) -> Result<Vec<Option<Value>>> {
    let items = sections(payload)?;
    if items.is_empty() {
        bail!("No sections found in payload");
    }
    if let Some(section_guid) = section_guid {
        return resolve_sections(payload, Some(&parse_multi_value(section_guid)));
    }
    resolve_sections(payload, None)
}

fn app_info_field_string(app_info: &Value, path: &[&str]) -> Result<String> {
    let value = nested_value(app_info, path)?;
    value_to_string(value)
}

fn app_info_field_i64(app_info: &Value, path: &[&str]) -> Result<i64> {
    let value = nested_value(app_info, path)?;
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
        .context(format!("missing integer field: {}", path.join(".")))
}

fn section_field_string(section: &Value, key: &str) -> Result<String> {
    section
        .get(key)
        .context(format!("missing section field: {key}"))
        .and_then(value_to_string)
}

fn section_field_i64(section: &Value, key: &str) -> Result<i64> {
    let value = section.get(key).context(format!("missing section field: {key}"))?;
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
        .context(format!("invalid integer section field: {key}"))
}

fn nested_value<'a>(value: &'a Value, path: &[&str]) -> Result<&'a Value> {
    let mut current = value;
    for key in path {
        current = current
            .get(*key)
            .context(format!("missing field: {}", path.join(".")))?;
    }
    Ok(current)
}

fn value_to_string(value: &Value) -> Result<String> {
    if let Some(text) = value.as_str() {
        Ok(text.to_string())
    } else if let Some(number) = value.as_i64() {
        Ok(number.to_string())
    } else if let Some(number) = value.as_u64() {
        Ok(number.to_string())
    } else if let Some(number) = value.as_f64() {
        Ok(number.to_string())
    } else {
        bail!("value is not a string-compatible scalar: {value}")
    }
}

fn header_string(headers: &HeaderMap, key: &str) -> Option<String> {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn json_value_to_string_pairs(value: &Value) -> Result<Vec<(String, String)>> {
    let object = value
        .as_object()
        .context("form payload must be a JSON object")?;
    object
        .iter()
        .map(|(key, value)| Ok((key.clone(), value_to_string(value)?)))
        .collect()
}

fn client_app_info_with_guid(app_info: &Value) -> Result<&Value> {
    if app_info.get("app_guid").is_none() {
        bail!("app_info missing app_guid");
    }
    Ok(app_info)
}

fn summarize_response(status_code: u16, text: &str) -> String {
    if let Ok(payload) = serde_json::from_str::<Value>(text) {
        let status = payload.get("status");
        let code = payload.get("code");
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| payload.get("msg").and_then(Value::as_str))
            .unwrap_or("");

        let success = status
            .map(|value| {
                value == &json!(1)
                    || value == &json!("1")
                    || value == &json!(true)
            })
            .unwrap_or(false)
            && status_code < 400;

        if success {
            if message.is_empty() {
                return format!("Success (HTTP {status_code})");
            }
            return format!("Success (HTTP {status_code}): {message}");
        }

        let mut parts = vec![format!("Failed (HTTP {status_code})")];
        if let Some(status) = status {
            parts.push(format!("status={}", scalar_to_string(status)));
        }
        if let Some(code) = code {
            parts.push(format!("code={}", scalar_to_string(code)));
        }
        if !message.is_empty() {
            parts.push(format!("message={message}"));
        }
        return parts.join(", ");
    }

    if (200..400).contains(&status_code) {
        format!("Success (HTTP {status_code})")
    } else {
        format!("Failed (HTTP {status_code})")
    }
}

fn scalar_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}
