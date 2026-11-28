//! Saved bank accounts and outgoing bank cards (re-notes/52_bank_cards.md).

use anyhow::{anyhow, bail, Result};
use reqwest::header::{HeaderValue, CONTENT_TYPE, COOKIE};
use reqwest::{Client, Method, StatusCode};
use serde_json::{json, Value};

use crate::chat::{fold_vietnamese, pick_str};
use crate::session::{is_trusted_zalo_url, Session};

const SERVICE: &str = "zinstant_msg";
const PAGE_SIZE: u32 = 10;
const MAX_PAGES: u32 = 10;
const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
pub(crate) const DEFAULT_DELETE_ERROR: &str =
    "Không thể xóa số tài khoản mặc định khi còn số tài khoản khác.";

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
    Ok(banks.iter().map(Bank::from_wire).collect())
}

fn parse_account_page(value: &Value) -> Result<AccountPage> {
    let accounts = value["myBanks"]
        .as_array()
        .ok_or_else(|| anyhow!("Zalo trả về danh sách số tài khoản không hợp lệ."))?;
    Ok(AccountPage {
        accounts: accounts.iter().map(BankAccount::from_wire).collect(),
        total: wire_u64(&value["total"]).unwrap_or(accounts.len() as u64),
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

fn response_code(value: &Value) -> Result<Option<i64>> {
    value
        .get("error_code")
        .map(|code| {
            let code = code
                .as_i64()
                .or_else(|| code.as_str()?.parse().ok())
                .ok_or_else(|| anyhow!("Mã phản hồi từ Zalo không hợp lệ."))?;
            if code != 0 {
                bail!("Zalo từ chối yêu cầu (mã lỗi {code}).");
            }
            Ok(code)
        })
        .transpose()
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
            let response = request
                .send()
                .await
                .map_err(|_| anyhow!("Không nhận được phản hồi từ dịch vụ số tài khoản."))?;
            let status = response.status();
            // A 404 identifies a host that does not serve this path. Any other failure can
            // occur after the server processed a POST, so never try another host in that case.
            if status == StatusCode::NOT_FOUND {
                not_found = true;
                continue;
            }
            if status != StatusCode::OK {
                bail!("Dịch vụ số tài khoản trả về lỗi HTTP {}.", status.as_u16());
            }

            let bytes = crate::media_resources::read_bounded_response(response, RESPONSE_LIMIT)
                .await
                .map_err(|_| anyhow!("Không đọc được phản hồi từ Zalo."))?;
            let response = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow!("Phản hồi từ Zalo không hợp lệ."))?;
            let payload = response_payload(
                response,
                session.session_key_bytes(),
                method == Method::POST,
            )?;
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
            let result = self.list_accounts(page).await?;
            saved.accounts.extend(result.accounts);
            saved.has_multiple |= result.total > 1 || saved.accounts.len() > 1;
            if !result.has_more {
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
        assert!(page.accounts[2].account_id.is_empty());
        assert!(!page.accounts[2].is_default);
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
