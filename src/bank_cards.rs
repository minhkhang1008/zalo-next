//! Saved bank accounts and Zalo bank cards (re-notes/52_bank_cards.md and re-notes/54).

use anyhow::{anyhow, bail, Result};
use reqwest::header::{HeaderValue, CONTENT_TYPE, COOKIE};
use reqwest::{Client, Method, StatusCode, Url};
use serde_json::{json, Value};
use std::time::Duration;

use crate::chat::{fold_vietnamese, pick_str};
use crate::session::{is_trusted_zalo_url, Session};

const SERVICE: &str = "zinstant_msg";
const PAGE_SIZE: u32 = 10;
const MAX_PAGES: u32 = 10;
const MAX_PAGE_ROWS: usize = 50;
const MAX_SAVED_ACCOUNTS: usize = 200;
const MAX_BANKS: usize = 500;
const MAX_NAME_CHARS: usize = 200;
const MAX_LOGO_BYTES: usize = 1024;
const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const RECEIVED_CARD_LIMIT: usize = 256 * 1024;
pub(crate) const DEFAULT_DELETE_ERROR: &str =
    "Không thể xóa số tài khoản mặc định khi còn số tài khoản khác.";
pub(crate) const SEND_UNCONFIRMED: &str =
    "Có thể thẻ đã được gửi. Kiểm tra cuộc trò chuyện trước khi gửi lại.";

#[derive(Clone, Default)]
pub struct Bank {
    pub bin: u32,
    pub logo: String,
    pub name: String,
    pub short_name: String,
    pub search_keyword: String,
}

// Omit Debug so bank details cannot be inadvertently logged.
#[derive(Clone, Default)]
pub struct BankAccount {
    pub account_id: String,
    pub bin: u32,
    pub bank_number: String,
    pub holder_name: String,
    pub is_default: bool,
    pub bank_name: String,
    pub bank_logo: String,
}

// Omit Debug so bank details cannot be inadvertently logged.
pub struct ReceivedBankCard {
    pub bin: u32,
    pub bank_name: String,
    pub number: String,
    pub logo: Option<String>,
    pub qr_payload: String,
}

pub(crate) fn received_card_url(url: &str) -> Option<Url> {
    let url = Url::parse(url).ok()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && is_trusted_zalo_url(url.as_str()))
    .then_some(url)
}

pub(crate) async fn fetch_received_bank_card(url: &str) -> Result<ReceivedBankCard, String> {
    let url =
        received_card_url(url).ok_or_else(|| "Địa chỉ thẻ ngân hàng không hợp lệ.".to_owned())?;
    let download_error = || "Không tải được thẻ ngân hàng.".to_owned();
    let client = Client::builder()
        .cookie_store(false)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| download_error())?;
    let response = client.get(url).send().await.map_err(|_| download_error())?;
    let bytes = crate::media_resources::read_bounded_response(response, RECEIVED_CARD_LIMIT)
        .await
        .map_err(|_| download_error())?;
    std::str::from_utf8(&bytes)
        .ok()
        .and_then(parse_received_bank_card)
        .ok_or_else(|| "Nội dung thẻ ngân hàng không hợp lệ.".into())
}

fn parse_received_bank_card(html: &str) -> Option<ReceivedBankCard> {
    let mut tags = html
        .split('<')
        .skip(1)
        .filter_map(|part| part.split_once('>'));
    let (_, bin) = tags
        .clone()
        .find(|(tag, _)| tag_attr(tag, "id") == Some("bank_bin"))?;
    let bin = bin.trim();
    if bin.len() != 6 || !bin.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let bin = bin.parse().ok().filter(|bin| *bin != 0)?;
    tags.find(|(tag, _)| has_class(tag, "bank_info"))?;
    let (_, bank_name) = tags
        .by_ref()
        .take_while(|(tag, _)| !has_class(tag, "number_text"))
        .find(|(tag, text)| has_class(tag, "normal_text") && !text.trim().is_empty())?;
    let (_, number) = tags.find(|(tag, _)| has_class(tag, "number_text"))?;
    let number = number.trim();
    if !valid_bank_number(number) {
        return None;
    }

    Some(ReceivedBankCard {
        bin,
        bank_name: unescape_html_text(bank_name.trim()).into_owned(),
        number: number.into(),
        logo: received_logo(html),
        qr_payload: transfer_payload(html)
            .filter(|payload| valid_vietqr_payload(payload))
            .map_or_else(|| vietqr_payload(bin, number), str::to_owned),
    })
}

fn tag_attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let (_, mut attrs) = tag.split_once(char::is_whitespace)?;
    while !attrs.trim_start().is_empty() {
        attrs = attrs.trim_start();
        let name_end = attrs.find(|c: char| c == '=' || c.is_whitespace())?;
        let (key, rest) = attrs.split_at(name_end);
        attrs = rest.trim_start();
        let Some(rest) = attrs.strip_prefix('=') else {
            continue;
        };
        let value = rest.trim_start();
        let (value, rest) = match value.chars().next()? {
            quote @ ('"' | '\'') => value[1..].split_once(quote)?,
            _ => value.split_at(value.find(char::is_whitespace).unwrap_or(value.len())),
        };
        if key == name {
            return Some(value);
        }
        attrs = rest;
    }
    None
}

fn has_class(tag: &str, class: &str) -> bool {
    tag_attr(tag, "class").is_some_and(|value| value.split_whitespace().any(|c| c == class))
}

fn unescape_html_text(text: &str) -> std::borrow::Cow<'_, str> {
    quick_xml::escape::unescape_with(text, |entity| match entity {
        "nbsp" => Some("\u{a0}"),
        _ => quick_xml::escape::resolve_xml_entity(entity),
    })
    .unwrap_or(std::borrow::Cow::Borrowed(text))
}

fn received_logo(html: &str) -> Option<String> {
    html.match_indices("https://res-zalo.zadn.vn/")
        .find_map(|(start, _)| {
            let url = html[start..]
                .split(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '<' | '>'))
                .next()?;
            let url = received_card_url(&unescape_html_text(url))?;
            (url.path().contains("_LOGO_")).then(|| url.into())
        })
}

fn transfer_payload(html: &str) -> Option<&str> {
    html.match_indices("https://bankcard-action.zalo.me?action=transfer")
        .find_map(|(i, prefix)| {
            let query = &html[i + prefix.len()..];
            let content = query
                .strip_prefix("&amp;content=")
                .or_else(|| query.strip_prefix("&content="))?;
            content
                .split(|c: char| c.is_whitespace() || matches!(c, '&' | '"' | '\'' | '<' | '>'))
                .next()
        })
}

fn valid_vietqr_payload(payload: &str) -> bool {
    if !payload.starts_with("000201") || payload.len() > 512 {
        return false;
    }
    let Some((data, crc)) = payload.split_at_checked(payload.len() - 4) else {
        return false;
    };
    data.ends_with("6304")
        && crc.bytes().all(|b| b.is_ascii_hexdigit())
        && u16::from_str_radix(crc, 16).is_ok_and(|crc| crc16_ccitt_false(data.as_bytes()) == crc)
}

pub struct AccountPage {
    pub accounts: Vec<BankAccount>,
    pub total: u64,
    pub has_more: bool,
}

#[derive(Default)]
pub(crate) struct SavedBankAccounts {
    pub accounts: Vec<BankAccount>,
    pub has_multiple: bool,
}

impl SavedBankAccounts {
    /// Return whether pagination can continue within the local row budget.
    fn append_page(&mut self, page: AccountPage) -> bool {
        let remaining = MAX_SAVED_ACCOUNTS - self.accounts.len();
        self.accounts
            .extend(page.accounts.into_iter().take(remaining));
        self.has_multiple |= page.total > 1 || self.accounts.len() > 1;
        page.has_more && self.accounts.len() < MAX_SAVED_ACCOUNTS
    }
}

fn wire_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn wire_bool(value: &Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.as_str()?.parse().ok())
        .or_else(|| wire_u64(value).map(|number| number != 0))
}

fn wire_bin(value: &Value) -> u32 {
    wire_u64(&value["bin"])
        .and_then(|bin| u32::try_from(bin).ok())
        .unwrap_or_default()
}

fn valid_name_length(value: &str) -> bool {
    value.chars().nth(MAX_NAME_CHARS).is_none()
}

impl Bank {
    fn from_wire(value: &Value) -> Self {
        Self {
            bin: wire_bin(value),
            logo: pick_str(value, &["logo"]).unwrap_or_default(),
            name: pick_str(value, &["name"]).unwrap_or_default(),
            short_name: pick_str(value, &["short_name"]).unwrap_or_default(),
            search_keyword: pick_str(value, &["search_key_word"]).unwrap_or_default(),
        }
    }
}

impl BankAccount {
    fn from_wire(value: &Value) -> Self {
        Self {
            account_id: pick_str(value, &["account_id", "id"]).unwrap_or_default(),
            bin: wire_bin(value),
            bank_number: pick_str(value, &["bank_number"]).unwrap_or_default(),
            holder_name: pick_str(value, &["holder_name"]).unwrap_or_default(),
            is_default: wire_bool(&value["is_default"])
                .or_else(|| wire_bool(&value["default"]))
                .unwrap_or_default(),
            bank_name: pick_str(value, &["bank_name"]).unwrap_or_default(),
            bank_logo: pick_str(value, &["bank_logo"]).unwrap_or_default(),
        }
    }
}

fn parse_banks(value: &Value) -> Result<Vec<Bank>> {
    let banks = value["banks"]
        .as_array()
        .ok_or_else(|| anyhow!("Zalo trả về danh sách ngân hàng không hợp lệ."))?;
    Ok(banks
        .iter()
        .take(MAX_BANKS)
        .filter(|row| row["name_eng"].as_str().is_none_or(valid_name_length))
        .map(Bank::from_wire)
        .filter(|bank| {
            valid_name_length(&bank.name)
                && valid_name_length(&bank.short_name)
                && valid_name_length(&bank.search_keyword)
                && bank.logo.len() <= MAX_LOGO_BYTES
        })
        .collect())
}

fn parse_account_page(value: &Value) -> Result<AccountPage> {
    let accounts = value["myBanks"]
        .as_array()
        .ok_or_else(|| anyhow!("Zalo trả về danh sách số tài khoản không hợp lệ."))?;
    Ok(AccountPage {
        accounts: accounts
            .iter()
            .take(MAX_PAGE_ROWS)
            .map(BankAccount::from_wire)
            .filter(|account| {
                valid_bank_number(&account.bank_number)
                    && valid_name_length(&account.holder_name)
                    && valid_name_length(&account.bank_name)
                    && account.bank_logo.len() <= MAX_LOGO_BYTES
                    && account.account_id.len() <= 20
            })
            .collect(),
        total: wire_u64(&value["total"])
            .unwrap_or_default()
            .max(accounts.len() as u64),
        has_more: wire_bool(&value["hasMore"]).unwrap_or_default(),
    })
}

fn parse_created_account(value: &Value) -> Result<BankAccount> {
    if !value.is_object() {
        bail!("Zalo trả về thông tin số tài khoản không hợp lệ.");
    }
    Ok(BankAccount::from_wire(value))
}

/// Match Zalo Web's bank-number rule and retain leading zeroes.
pub fn valid_bank_number(value: &str) -> bool {
    (1..=19).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// Match Zalo Web's holder-name rule, including the single spaces between words.
pub fn valid_holder_name(value: &str) -> bool {
    (5..=50).contains(&value.len())
        && value
            .split(' ')
            .all(|word| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

/// Fold both precomposed and decomposed Vietnamese accents and preserve input spacing.
pub fn upper_no_diacritics(value: &str) -> String {
    let without_combining_marks: String = value
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{300}'
                    | '\u{301}'
                    | '\u{303}'
                    | '\u{309}'
                    | '\u{323}'
                    | '\u{302}'
                    | '\u{306}'
                    | '\u{31b}'
            )
        })
        .collect();
    fold_vietnamese(&without_combining_marks).to_uppercase()
}

fn list_params(page: u32) -> Value {
    json!({ "page": page, "limit": PAGE_SIZE })
}

fn write_params(account: &BankAccount, update: bool) -> Result<Value> {
    if account.bin == 0
        || !valid_bank_number(&account.bank_number)
        || !valid_holder_name(&account.holder_name)
    {
        bail!("Thông tin số tài khoản không hợp lệ.");
    }
    let mut params = json!({
        "bin": account.bin,
        "bank_number": account.bank_number,
        "holder_name": account.holder_name,
        "language": "vi",
    });
    if update {
        params["account_id"] = json!(numeric_account_id(account)?);
    }
    Ok(params)
}

fn numeric_account_id(account: &BankAccount) -> Result<u64> {
    account
        .account_id
        .parse()
        .map_err(|_| anyhow!("Mã số tài khoản đã lưu không hợp lệ."))
}

fn delete_params(account: &BankAccount) -> Result<Value> {
    Ok(json!({
        "account_id": numeric_account_id(account)?,
        "is_default": account.is_default,
        "language": "vi",
    }))
}

fn card_params(
    thread_id: &str,
    is_group: bool,
    account: &BankAccount,
    now_ms: i64,
) -> Result<Value> {
    let dest_uid = if is_group {
        let group = thread_id.strip_prefix("group_").unwrap_or(thread_id);
        group.strip_prefix(['g', 'G']).unwrap_or(group)
    } else {
        thread_id
    };
    if dest_uid.is_empty()
        || account.bin == 0
        || account.bank_number.is_empty()
        || account.holder_name.is_empty()
    {
        bail!("Thiếu thông tin để gửi số tài khoản.");
    }
    Ok(json!({
        "binBank": account.bin,
        "numAccBank": account.bank_number,
        "nameAccBank": account.holder_name,
        "cliMsgId": now_ms,
        "tsMsg": now_ms,
        "destUid": dest_uid,
        "destType": u8::from(is_group),
    }))
}

#[derive(Debug)]
struct ZaloRejection(i64);

impl std::fmt::Display for ZaloRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Zalo từ chối yêu cầu (mã lỗi {}).", self.0)
    }
}

impl std::error::Error for ZaloRejection {}

fn response_code(value: &Value) -> Result<Option<i64>> {
    value
        .get("error_code")
        .map(|code| {
            let code = code
                .as_i64()
                .or_else(|| code.as_str()?.parse().ok())
                .ok_or_else(|| anyhow!("Mã phản hồi từ Zalo không hợp lệ."))?;
            if code != 0 {
                return Err(ZaloRejection(code).into());
            }
            Ok(code)
        })
        .transpose()
}

fn card_ack_error(path: &str, error: anyhow::Error) -> anyhow::Error {
    if path == "card" && !error.is::<ZaloRejection>() {
        anyhow!(SEND_UNCONFIRMED)
    } else {
        error
    }
}

fn response_payload(response: Value, key: &[u8], require_success_code: bool) -> Result<Value> {
    let code = response_code(&response)?;
    if require_success_code && code.is_none() {
        bail!("Zalo chưa xác nhận yêu cầu.");
    }
    let Some(data) = response.get("data") else {
        return Ok(response);
    };
    let Some(text) = data.as_str().filter(|text| !text.is_empty()) else {
        response_code(data)?;
        return Ok(data.clone());
    };
    // The service can return a plain object, an encrypted object, or an encrypted envelope.
    let decoded: Value = serde_json::from_str(text)
        .or_else(|_| {
            let plaintext = crate::crypto::decode_aes(key, text).ok();
            serde_json::from_str(plaintext.as_deref().unwrap_or(""))
        })
        .map_err(|_| anyhow!("Không đọc được phản hồi từ Zalo."))?;
    response_code(&decoded)?;
    Ok(decoded.get("data").cloned().unwrap_or(decoded))
}

/// Standard post-login AES requests without response logging or automatic retries.
pub struct BankCards<'a> {
    session: &'a Session,
    http: Client,
}

impl<'a> BankCards<'a> {
    pub fn new(session: &'a Session) -> Result<Self> {
        if session.login.is_none() || session.zpw_sek.as_deref().is_none_or(str::is_empty) {
            bail!("Chưa đăng nhập Zalo.");
        }
        let http = session
            .bank_http_client()
            .map_err(|_| anyhow!("Không thể kết nối dịch vụ số tài khoản."))?;
        Ok(Self { session, http })
    }

    async fn request(&self, method: Method, path: &str, params: Value) -> Result<Value> {
        let session = self.session;
        let params = session
            .encoded_params(&params)
            .map_err(|_| anyhow!("Không thể mã hóa yêu cầu số tài khoản."))?;
        let sek = session.zpw_sek.as_deref().unwrap_or_default();
        let mut cookie = HeaderValue::from_str(&format!("zpw_sek={sek}; _zlang=vi"))
            .map_err(|_| anyhow!("Phiên đăng nhập Zalo không hợp lệ."))?;
        let mut token = HeaderValue::from_str(sek)
            .map_err(|_| anyhow!("Phiên đăng nhập Zalo không hợp lệ."))?;
        cookie.set_sensitive(true);
        token.set_sensitive(true);

        let query = session.common_params();
        let mut not_found = false;
        for host in session.domains.candidates(SERVICE) {
            let mut url = format!("{host}/api/transfer/{path}?{query}");
            if method == Method::GET {
                url.push_str("&params=");
                url.push_str(&params);
            }
            if !is_trusted_zalo_url(&url) {
                continue;
            }

            let mut request = self
                .http
                .request(method.clone(), url)
                .header(COOKIE, cookie.clone())
                .header("zpw_sek", token.clone());
            if method == Method::POST {
                request = request
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(format!("params={params}"));
            }
            let response = request.send().await.map_err(|_| {
                card_ack_error(
                    path,
                    anyhow!("Không nhận được phản hồi từ dịch vụ số tài khoản."),
                )
            })?;
            let status = response.status();
            // A 404 identifies a host that does not serve this path. Any other failure can
            // occur after the server processed a POST, so never try another host in that case.
            if status == StatusCode::NOT_FOUND {
                not_found = true;
                continue;
            }
            if status != StatusCode::OK {
                let error = anyhow!("Dịch vụ số tài khoản trả về lỗi HTTP {}.", status.as_u16());
                if status.is_client_error() && status != StatusCode::REQUEST_TIMEOUT {
                    return Err(error);
                }
                return Err(card_ack_error(path, error));
            }

            let bytes = crate::media_resources::read_bounded_response(response, RESPONSE_LIMIT)
                .await
                .map_err(|_| card_ack_error(path, anyhow!("Không đọc được phản hồi từ Zalo.")))?;
            let response = serde_json::from_slice(&bytes)
                .map_err(|_| card_ack_error(path, anyhow!("Phản hồi từ Zalo không hợp lệ.")))?;
            let payload = response_payload(
                response,
                session.session_key_bytes(),
                method == Method::POST,
            )
            .map_err(|error| card_ack_error(path, error))?;
            session.domains.remember_winner(SERVICE, &host);
            return Ok(payload);
        }

        if not_found {
            bail!("Không tìm thấy dịch vụ số tài khoản (HTTP 404).");
        }
        bail!("Không có máy chủ số tài khoản Zalo hợp lệ.");
    }

    pub async fn list_banks(&self) -> Result<Vec<Bank>> {
        parse_banks(&self.request(Method::GET, "conf", json!({})).await?)
    }

    pub async fn list_accounts(&self, page: u32) -> Result<AccountPage> {
        parse_account_page(&self.request(Method::GET, "list", list_params(page)).await?)
    }

    pub(crate) async fn list_all_accounts(&self) -> Result<SavedBankAccounts> {
        let mut saved = SavedBankAccounts::default();
        for page in 0..MAX_PAGES {
            if !saved.append_page(self.list_accounts(page).await?) {
                break;
            }
            if page == MAX_PAGES - 1 {
                saved.has_multiple = true;
            }
        }
        Ok(saved)
    }

    pub async fn create_account(&self, account: &BankAccount) -> Result<BankAccount> {
        let params = write_params(account, false)?;
        parse_created_account(&self.request(Method::POST, "create", params).await?)
    }

    pub async fn update_account(&self, account: &BankAccount) -> Result<()> {
        let params = write_params(account, true)?;
        self.request(Method::POST, "update", params).await?;
        Ok(())
    }

    pub async fn delete_account(&self, account: &BankAccount) -> Result<()> {
        self.request(Method::POST, "delete", delete_params(account)?)
            .await?;
        Ok(())
    }

    pub async fn send_bank_card(
        &self,
        thread_id: &str,
        is_group: bool,
        account: &BankAccount,
    ) -> Result<()> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.request(
            Method::POST,
            "card",
            card_params(thread_id, is_group, account, now_ms)?,
        )
        .await?;
        Ok(())
    }
}

fn tlv(tag: &str, value: &str) -> String {
    format!("{tag}{:02}{value}", value.len())
}

fn crc16_ccitt_false(bytes: &[u8]) -> u16 {
    let mut crc = 0xffffu16;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            let high_bit_set = crc & 0x8000 != 0;
            crc <<= 1;
            if high_bit_set {
                crc ^= 0x1021;
            }
        }
    }
    crc
}

/// Build a static NAPAS VietQR payload for a valid bank BIN and account number.
pub fn vietqr_payload(bin: u32, account: &str) -> String {
    let recipient = tlv("00", &bin.to_string()) + &tlv("01", account);
    let merchant = tlv("00", "A000000727") + &tlv("01", &recipient) + &tlv("02", "QRIBFTTA");
    let payload = format!("000201010211{}53037045802VN6304", tlv("38", &merchant));
    format!("{payload}{:04X}", crc16_ccitt_false(payload.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD_HTML: &str = include_str!("../tests/fixtures/zinstant_bankcard_pc.html");
    const CARD_QR: &str = concat!(
        "00020101021138540010A00000072701240006546034011001234567890208QRIBFTTA",
        "53037045802VN80300012com.vng.zalo0110ZABANKCARD6304"
    );

    #[test]
    fn received_bank_cards_parse_the_redacted_fixture() {
        let card = parse_received_bank_card(CARD_HTML).unwrap();
        assert_eq!(card.bin, 546034);
        assert_eq!(card.number, "0123456789");
        assert_eq!(card.bank_name, "Ngân hàng CAKE by VPBank");
        assert_eq!(
            card.logo.as_deref(),
            Some("https://res-zalo.zadn.vn/upload/media/2022/9/22/546034_LOGO_1663814241511_1282314.png")
        );
        assert_eq!(
            transfer_payload(CARD_HTML),
            Some(format!("{CARD_QR}0D69").as_str())
        );
    }

    #[test]
    fn received_bank_cards_accept_valid_crc_and_fall_back_on_bad_crc() {
        // The redacted fixture retains a CRC that does not match its synthetic account.
        assert_eq!(
            parse_received_bank_card(CARD_HTML).unwrap().qr_payload,
            vietqr_payload(546034, "0123456789")
        );
        let html = CARD_HTML.replacen("63040D69", "630455DB", 1);
        let valid = format!("{CARD_QR}55DB");
        assert!(valid_vietqr_payload(&valid));
        assert!(!valid_vietqr_payload(&format!("{CARD_QR}0D69")));
        assert!(!valid_vietqr_payload("000201"));
        let oversized = format!("000201{}6304", "0".repeat(500));
        let oversized_crc = crc16_ccitt_false(oversized.as_bytes());
        assert!(!valid_vietqr_payload(&format!(
            "{oversized}{oversized_crc:04X}"
        )));
        assert_eq!(parse_received_bank_card(&html).unwrap().qr_payload, valid);
    }

    #[test]
    fn received_bank_cards_unescape_html_and_reject_invalid_numbers() {
        let html = CARD_HTML.replace("CAKE by VPBank", "CAKE &amp; VPBank &#x26; Co");
        assert_eq!(
            parse_received_bank_card(&html).unwrap().bank_name,
            "Ngân hàng CAKE & VPBank & Co"
        );
        for number in ["", "01-23456789", "12345678901234567890"] {
            let html = CARD_HTML.replace(">0123456789<", &format!(">{number}<"));
            assert!(parse_received_bank_card(&html).is_none());
        }
    }

    #[test]
    fn parses_catalogue_and_defaults_missing_fields() {
        let banks = parse_banks(&json!({ "banks": [{
            "bin": "970436", "name": "Ngân hàng mẫu", "short_name": "TEST",
            "search_key_word": "bank", "logo": 1
        }, {}] }))
        .unwrap();
        assert_eq!(banks[0].bin, 970436);
        assert_eq!(banks[0].name, "Ngân hàng mẫu");
        assert_eq!(banks[0].short_name, "TEST");
        assert_eq!(banks[0].search_keyword, "bank");
        assert_eq!(banks[0].logo, "1");
        assert_eq!(banks[1].bin, 0);
        assert!(banks[1].name.is_empty());
        assert!(parse_banks(&json!({})).is_err());
    }

    #[test]
    fn decrypts_list_page_with_loose_wire_types() {
        let key = [7; 32];
        let wire = json!({
            "myBanks": [
                { "id": 2, "account_id": 42, "bin": "970436", "bank_number": "001234",
                  "bank_name": "TEST", "bank_logo": "logo", "holder_name": "NGUYEN VAN DUC",
                  "is_default": "0", "default": 1 },
                { "id": 7, "bank_number": 100, "holder_name": 12345, "default": true }, {}
            ],
            "total": "17", "hasMore": "1",
        });
        let encrypted = crate::crypto::encode_aes(&key, &wire.to_string()).unwrap();
        let payload =
            response_payload(json!({"error_code": 0, "data": encrypted}), &key, false).unwrap();
        let page = parse_account_page(&payload).unwrap();
        assert!(page.has_more);
        assert_eq!(page.total, 17);
        assert_eq!(page.accounts[0].account_id, "42");
        assert_eq!(page.accounts[0].bin, 970436);
        assert_eq!(page.accounts[0].bank_number, "001234");
        assert_eq!(page.accounts[0].bank_name, "TEST");
        assert_eq!(page.accounts[0].bank_logo, "logo");
        assert_eq!(page.accounts[0].holder_name, "NGUYEN VAN DUC");
        assert!(!page.accounts[0].is_default);
        assert_eq!(page.accounts[1].account_id, "7");
        assert_eq!(page.accounts[1].bank_number, "100");
        assert_eq!(page.accounts[1].holder_name, "12345");
        assert!(page.accounts[1].is_default);
        assert_eq!(page.accounts.len(), 2);
    }

    #[test]
    fn caps_bank_catalogue_rows() {
        let banks: Vec<_> = (1..=501).map(|bin| json!({"bin": bin})).collect();
        let parsed = parse_banks(&json!({"banks": banks})).unwrap();
        assert_eq!(parsed.len(), 500);
        assert_eq!(parsed.last().unwrap().bin, 500);
    }

    #[test]
    fn caps_bank_account_page_rows() {
        let accounts: Vec<_> = (1..=51)
            .map(|id| json!({"id": id, "bank_number": "00001"}))
            .collect();
        let parsed = parse_account_page(&json!({"myBanks": accounts, "hasMore": true})).unwrap();
        assert_eq!(parsed.accounts.len(), 50);
        assert_eq!(parsed.accounts.last().unwrap().account_id, "50");
        assert_eq!(parsed.total, 51);
        assert!(parsed.has_more);
    }

    #[test]
    fn caps_saved_bank_accounts_across_pages() {
        let mut saved = SavedBankAccounts::default();
        let mut next_id = 1;
        for (page, count) in [7, 50, 50, 50, 50].into_iter().enumerate() {
            let accounts: Vec<_> = (next_id..next_id + count)
                .map(|id| json!({"id": id, "bank_number": "00001"}))
                .collect();
            next_id += count;
            let parsed =
                parse_account_page(&json!({"myBanks": accounts, "hasMore": true})).unwrap();
            assert_eq!(saved.append_page(parsed), page < 4);
        }
        assert_eq!(saved.accounts.len(), 200);
        assert_eq!(saved.accounts.last().unwrap().account_id, "200");
        assert!(saved.has_multiple);
    }

    #[test]
    fn marks_unreadable_bank_card_acknowledgements_but_keeps_explicit_rejections() {
        for response in [
            json!({}),
            json!({"error_code": "invalid"}),
            json!({"error_code": 0, "data": "invalid"}),
        ] {
            let error = response_payload(response, &[0; 32], true).unwrap_err();
            assert_eq!(card_ack_error("card", error).to_string(), SEND_UNCONFIRMED);
        }
        for response in [
            json!({"error_code": 2}),
            json!({"error_code": 0, "data": {"error_code": 2}}),
        ] {
            let error = response_payload(response, &[], true).unwrap_err();
            assert_eq!(
                card_ack_error("card", error).to_string(),
                "Zalo từ chối yêu cầu (mã lỗi 2)."
            );
        }
    }

    #[test]
    fn drops_oversized_bank_catalogue_fields() {
        let parsed = parse_banks(&json!({"banks": [
            {"bin": 1, "name": "Á".repeat(200), "logo": "a".repeat(1024)},
            {"bin": 2, "name": "Á".repeat(201)},
            {"bin": 3, "short_name": "a".repeat(201)},
            {"bin": 4, "search_key_word": "a".repeat(201)},
            {"bin": 5, "logo": "a".repeat(1025)}
        ]}))
        .unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].bin, 1);
    }

    #[test]
    fn drops_oversized_or_invalid_bank_account_fields() {
        let parsed = parse_account_page(&json!({"myBanks": [
            {"id": 1, "bank_number": "00001", "holder_name": "Á".repeat(200),
             "bank_name": "Á".repeat(200), "bank_logo": "a".repeat(1024)},
            {"id": 2, "bank_number": "00001", "holder_name": "Á".repeat(201)},
            {"id": 3, "bank_number": "00001", "bank_name": "Á".repeat(201)},
            {"id": 4, "bank_number": "00001", "bank_logo": "a".repeat(1025)},
            {"id": 5, "bank_number": "a".repeat(20)},
            {"id": 6, "bank_number": "01 23"}
        ]}))
        .unwrap();
        assert_eq!(parsed.accounts.len(), 1);
        assert_eq!(parsed.accounts[0].account_id, "1");
        assert_eq!(parsed.total, 6);
    }

    #[test]
    fn parses_create_response_and_requires_explicit_post_success() {
        let result = response_payload(
            json!({"error_code": "0", "data": {
                "account_id": "77", "bank_number": 12345, "bin": 970436,
                "holder_name": "NGUYEN VAN DUC", "is_default": 1
            }}),
            &[],
            true,
        )
        .unwrap();
        let account = parse_created_account(&result).unwrap();
        assert_eq!(account.account_id, "77");
        assert_eq!(account.bank_number, "12345");
        assert_eq!(account.bin, 970436);
        assert_eq!(account.holder_name, "NGUYEN VAN DUC");
        assert!(account.is_default);
        assert!(account.bank_logo.is_empty());
        assert!(response_payload(json!({}), &[], true).is_err());
        assert!(response_payload(json!({"error_code": "2"}), &[], true).is_err());
        assert!(response_payload(
            json!({"error_code": 0, "data": {"error_code": 2}}),
            &[],
            true
        )
        .is_err());
    }

    #[test]
    fn validates_bank_number_and_name_like_web() {
        for accepted in ["0", "Ab01", "1234567890123456789"] {
            assert!(valid_bank_number(accepted));
        }
        for rejected in [
            "",
            "12345678901234567890",
            "01 23",
            "AB-12",
            "１２",
            "123\n",
        ] {
            assert!(!valid_bank_number(rejected));
        }
        for accepted in ["ABCDE", "a1 B2", "NGUYEN VAN DUC", &"A".repeat(50)] {
            assert!(valid_holder_name(accepted));
        }
        for rejected in [
            "ABCD",
            " ANH TU",
            "ANH TU ",
            "ANH  TU",
            "ANH\tTU",
            "ÁNH TU",
            "ANH-TU",
            &"A".repeat(51),
        ] {
            assert!(!valid_holder_name(rejected));
        }
    }

    #[test]
    fn folds_vietnamese_holder_name_without_rewriting_spaces() {
        assert_eq!(upper_no_diacritics("Nguyễn Văn Đức"), "NGUYEN VAN DUC");
        assert_eq!(
            upper_no_diacritics("Nguye\u{302}\u{303}n Va\u{306}n Đu\u{31b}\u{301}c"),
            "NGUYEN VAN DUC"
        );
        assert_eq!(upper_no_diacritics(" Đặng  Thị "), " DANG  THI ");
    }

    #[test]
    fn request_params_match_transfer_endpoints() {
        let account = BankAccount {
            account_id: "101".into(),
            bin: 970436,
            bank_number: "0012345".into(),
            holder_name: "NGUYEN VAN DUC".into(),
            is_default: true,
            ..Default::default()
        };
        assert_eq!(list_params(0), json!({"page": 0, "limit": 10}));
        assert_eq!(
            write_params(&account, false).unwrap(),
            json!({
                "bin": 970436, "bank_number": "0012345", "holder_name": "NGUYEN VAN DUC", "language": "vi"
            })
        );
        assert_eq!(
            write_params(&account, true).unwrap(),
            json!({
                "account_id": 101, "bin": 970436, "bank_number": "0012345",
                "holder_name": "NGUYEN VAN DUC", "language": "vi"
            })
        );
        assert_eq!(
            delete_params(&account).unwrap(),
            json!({
                "account_id": 101, "is_default": true, "language": "vi"
            })
        );
        let now_ms = 1_800_000_000_000;
        assert_eq!(
            card_params("123456", false, &account, now_ms).unwrap(),
            json!({
                "binBank": 970436, "numAccBank": "0012345", "nameAccBank": "NGUYEN VAN DUC",
                "cliMsgId": now_ms, "tsMsg": now_ms, "destUid": "123456", "destType": 0
            })
        );
        for group in ["123456", "g123456", "G123456", "group_123456"] {
            let params = card_params(group, true, &account, now_ms).unwrap();
            assert_eq!(params["destUid"], "123456");
            assert_eq!(params["destType"], 1);
        }
        assert_eq!(
            card_params("g123456", false, &account, now_ms).unwrap()["destUid"],
            "g123456"
        );
    }

    #[test]
    fn vietqr_static_payload_and_crc() {
        assert_eq!(crc16_ccitt_false(b"123456789"), 0x29B1);
        // BIN field takes 10 bytes (00 + 06 + BIN), account takes 13; tag 01
        // wraps those 23 bytes. Tag 38 contains 14 + 27 + 12 = 53 bytes.
        // Starting at FFFF, XOR each ASCII byte into the high octet and shift
        // left eight times, XORing 1021 whenever the bit shifted out is 1.
        // CRC is 0449 after tags 00/01, 2678 after tag 38, and 5330 through 6304.
        assert_eq!(
            vietqr_payload(970436, "123456789"),
            concat!(
                "000201010211",
                "38530010A0000007270123000697043601091234567890208QRIBFTTA",
                "53037045802VN6304",
                "5330"
            )
        );
    }
}
