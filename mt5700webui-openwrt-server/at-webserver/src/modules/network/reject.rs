//! `^REJINFO` decoder (manual 13.14/13.14.2/13.14.3) — registration, service
//! request or network detach rejections reported by the network.
//!
//! The WebUI's settings page used to own this: a table of cause values, the
//! domain/rat/reject-type labels and the "USIM authentication failure" range,
//! decoded from the raw URC in `modem/reject.ts`. That made the handbook a
//! frontend table, so a LuCI page wanting the same text would have copied it a
//! second time. The module owns it now: the URC path decodes the line once and
//! publishes `network.reject`, and the page renders the object.
//!
//! ```text
//! ^REJINFO:<PLMN ID>,<Service Domain>,<Reject Cause>,<Rat Type>,<Reject Type>,
//!          <Original Reject Cause>,<Lac>,<Rac>,<Cell Id>[,<Esm Reject Cause>]
//! ```

use crate::core::json::{self, Value};
use std::collections::BTreeMap;

/// 手册 13.14.3 `<Service Domain>`.
fn domain_text(domain: i64) -> String {
    match domain {
        0 => "CS 域".to_string(),
        1 => "PS 域".to_string(),
        2 => "CS+PS 域".to_string(),
        other => format!("域 {}", other),
    }
}

/// 手册 13.14.3 `<Rat Type>`.
fn rat_text(rat: i64) -> String {
    match rat {
        0 => "GERAN(2G)".to_string(),
        1 => "UTRAN(3G)".to_string(),
        2 => "E-UTRAN(4G)".to_string(),
        5 => "NR-5GC(5G SA)".to_string(),
        6 => "其他".to_string(),
        other => format!("制式 {}", other),
    }
}

/// 手册 13.14.3 `<Reject Type>`.
fn reject_type_text(kind: i64) -> String {
    match kind {
        0 => "LAU 被拒".to_string(),
        1 => "鉴权失败".to_string(),
        2 => "业务请求被拒".to_string(),
        3 => "网络 detach 被拒".to_string(),
        4 => "ATTACH 被拒".to_string(),
        5 => "RAU 被拒".to_string(),
        6 => "TAU 被拒".to_string(),
        other => format!("类型 {}", other),
    }
}

/// 3GPP TS 24.008 / 24.301 / 24.501 cause values the manual refers to (it does
/// not list them itself), plus the module's own extension values (13.14.2).
const CAUSES: &[(i64, &str)] = &[
    (2, "IMSI 未在 HSS 登记"),
    (3, "非法终端"),
    (5, "IMEI 不被接受"),
    (6, "非法设备"),
    (7, "不允许使用分组域业务"),
    (8, "不允许使用分组域和非分组域业务"),
    (9, "网络无法识别终端身份"),
    (10, "已被网络隐式分离"),
    (11, "不允许使用该 PLMN"),
    (12, "不允许在该跟踪区注册"),
    (13, "该跟踪区不允许漫游"),
    (14, "该 PLMN 不提供分组域业务"),
    (15, "跟踪区内没有合适的小区"),
    (16, "MSC 暂时不可达"),
    (17, "网络故障"),
    (18, "CS 域不可用"),
    (19, "ESM 流程失败"),
    (20, "MAC 校验失败"),
    (21, "同步失败"),
    (22, "网络拥塞"),
    (23, "终端安全能力不匹配"),
    (24, "安全模式被拒绝"),
    (25, "未授权接入该 CSG"),
    (26, "非 EPS 鉴权不可接受"),
    (27, "不允许使用 N1 模式"),
    (28, "受限的服务区域"),
    (31, "需要重定向到 4G 核心网"),
    (35, "该 PLMN 未授权所请求的业务"),
    (39, "CS 业务暂时不可用"),
    (40, "没有激活的 EPS 承载"),
    (42, "严重网络故障"),
    (43, "LADN 不可用"),
    (62, "没有可用的网络切片"),
    (65, "已达到 PDU 会话数量上限"),
    (67, "切片与 DNN 资源不足"),
    (71, "不允许通过非 3GPP 接入 5G 核心网"),
    (72, "服务网络未授权"),
    (95, "消息语义错误"),
    (96, "必选信元无效"),
    (97, "消息类型不存在或未实现"),
    (98, "消息类型与协议状态不匹配"),
    (99, "信元不存在或未实现"),
    (100, "条件信元错误"),
    (101, "消息与协议状态不匹配"),
    (111, "协议错误"),
    (256, "鉴权失败（模组内部扩展）"),
    (258, "联合注册中 CS 失败（其他原因）"),
    (301, "CS/PS 注册网络无响应"),
    (302, "CS/PS 注册建链异常"),
    (303, "CS/PS 注册建链异常"),
];

/// 手册 13.14.2: USIM authentication failures use 65537..=65543.
const USIM_CAUSE_MIN: i64 = 65537;
const USIM_CAUSE_MAX: i64 = 65543;

/// The cause's Chinese text; unknown values keep their number.
pub fn cause_text(cause: i64) -> String {
    if let Some((_, text)) = CAUSES.iter().find(|(value, _)| *value == cause) {
        return text.to_string();
    }
    if (USIM_CAUSE_MIN..=USIM_CAUSE_MAX).contains(&cause) {
        return format!("USIM 鉴权失败（#{}）", cause);
    }
    format!("未知原因（#{}）", cause)
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').trim().to_string()
}

/// A hex/number field: the first field wins, anything unparsable is 0 (what the
/// page's `Number(...) || 0` did).
fn num(value: &str) -> i64 {
    unquote(value).parse().unwrap_or(0)
}

/// Parse one `^REJINFO` line into the object the settings page renders.
///
/// The handbook body prints the separator as a full-width colon in places, so
/// both `^REJINFO:` and `^REJINFO：` are accepted. Fewer than six fields is not
/// a rejection report (`None`), matching the old page parser.
pub fn parse(line: &str) -> Option<Value> {
    let at = line.find("^REJINFO")?;
    let rest = line[at + "^REJINFO".len()..].trim_start();
    let rest = rest.trim_start_matches(|c| c == ':' || c == '：');
    let fields: Vec<String> = rest.split(',').map(unquote).collect();
    if fields.len() < 6 {
        return None;
    }

    let cause = num(&fields[2]);
    let rat = num(&fields[3]);
    let domain = num(&fields[1]);
    let reject_type = num(&fields[4]);
    let field = |index: usize| fields.get(index).cloned().unwrap_or_default();

    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    m.insert("plmn".to_string(), json::str_val(&fields[0]));
    m.insert("domain".to_string(), json::num_val(domain));
    m.insert("domainText".to_string(), json::str_val(&domain_text(domain)));
    m.insert("cause".to_string(), json::num_val(cause));
    m.insert("causeText".to_string(), json::str_val(&cause_text(cause)));
    m.insert("rat".to_string(), json::num_val(rat));
    m.insert("ratText".to_string(), json::str_val(&rat_text(rat)));
    m.insert("rejectType".to_string(), json::num_val(reject_type));
    m.insert(
        "rejectTypeText".to_string(),
        json::str_val(&reject_type_text(reject_type)),
    );
    m.insert("originalCause".to_string(), json::num_val(num(&fields[5])));
    m.insert("lac".to_string(), json::str_val(&field(6)));
    m.insert("rac".to_string(), json::str_val(&field(7)));
    m.insert("cellId".to_string(), json::str_val(&field(8)));
    // 手册：只有 LNAS 注册被拒 #19 时才带这个值。
    if let Some(esm) = fields.get(9) {
        m.insert("esmCause".to_string(), json::num_val(num(esm)));
    }
    m.insert("raw".to_string(), json::str_val(line.trim()));
    m.insert(
        "at".to_string(),
        json::num_val(crate::core::runtime::now_ms() as i64),
    );
    Some(Value::Obj(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manual_example_decodes_to_the_pages_text() {
        // 手册 13.14 的例子：^REJINFO:46000,1,40,2,3,40,"0026F8","FF","0A444202"
        let v = parse("^REJINFO:46000,1,40,2,3,40,\"0026F8\",\"FF\",\"0A444202\"").unwrap();
        assert_eq!(v.get("plmn").and_then(|x| x.as_str()), Some("46000"));
        assert_eq!(v.get("domain").and_then(|x| x.as_i64()), Some(1));
        assert_eq!(v.get("domainText").and_then(|x| x.as_str()), Some("PS 域"));
        assert_eq!(v.get("cause").and_then(|x| x.as_i64()), Some(40));
        assert_eq!(
            v.get("causeText").and_then(|x| x.as_str()),
            Some("没有激活的 EPS 承载")
        );
        assert_eq!(v.get("ratText").and_then(|x| x.as_str()), Some("E-UTRAN(4G)"));
        assert_eq!(
            v.get("rejectTypeText").and_then(|x| x.as_str()),
            Some("网络 detach 被拒")
        );
        assert_eq!(v.get("lac").and_then(|x| x.as_str()), Some("0026F8"));
        assert_eq!(v.get("cellId").and_then(|x| x.as_str()), Some("0A444202"));
        // 9 段时没有 ESM 原因
        assert!(v.get("esmCause").is_none());
        assert!(v.get("at").and_then(|x| x.as_i64()).unwrap_or(0) > 0);
    }

    #[test]
    fn unknown_values_keep_their_numbers_and_short_lines_are_rejected() {
        assert_eq!(cause_text(40), "没有激活的 EPS 承载");
        assert_eq!(cause_text(65537), "USIM 鉴权失败（#65537）");
        assert_eq!(cause_text(65543), "USIM 鉴权失败（#65543）");
        assert_eq!(cause_text(444), "未知原因（#444）");
        assert_eq!(domain_text(3), "域 3");
        assert_eq!(rat_text(9), "制式 9");
        assert_eq!(reject_type_text(7), "类型 7");

        assert!(parse("^REJINFO:46000,1,40").is_none());
        assert!(parse("+CREG: 2,1").is_none());
        // 全角冒号（手册正文的写法）照样认。
        let v = parse("^REJINFO：46000,2,13,0,0,13").unwrap();
        assert_eq!(v.get("domainText").and_then(|x| x.as_str()), Some("CS+PS 域"));
        assert_eq!(v.get("ratText").and_then(|x| x.as_str()), Some("GERAN(2G)"));
        assert_eq!(v.get("lac").and_then(|x| x.as_str()), Some(""));
        // 第 10 段存在（哪怕是空）就带上 esmCause
        let v = parse("^REJINFO:46000,1,19,2,4,19,\"0026F8\",\"FF\",\"0A444202\",").unwrap();
        assert_eq!(v.get("esmCause").and_then(|x| x.as_i64()), Some(0));
    }
}
