//! Native IP parsing and inclusive range mathematics for net.BlockList.
use serde_json::{json, Value};
use std::net::IpAddr;
fn address(value: &Value, family: &str) -> Result<(u128, String, u32), String> {
    let text = value.as_str().ok_or("address must be a string")?;
    if family == "ipv4" && text.contains('%') {
        return Err("Invalid socket address".into());
    }
    let text = text.split('%').next().unwrap_or(text);
    let ip: IpAddr = text.parse().map_err(|_| "Invalid socket address")?;
    match (ip, family) {
        (IpAddr::V4(ip), "ipv4") => Ok((u128::from(ip.to_ipv6_mapped()), ip.to_string(), 32)),
        (IpAddr::V6(ip), "ipv6") => Ok((u128::from(ip), ip.to_string(), 128)),
        _ => Err("Invalid address family".into()),
    }
}
fn matches(rule: &Value, value: u128) -> bool {
    let low = rule["low"].as_str().and_then(|v| v.parse::<u128>().ok());
    let high = rule["high"].as_str().and_then(|v| v.parse::<u128>().ok());
    matches!((low, high), (Some(low), Some(high)) if low <= value && value <= high)
}
fn subnet(value: u128, prefix: &Value, width: u32) -> Result<(u128, u128), String> {
    let prefix = prefix.as_u64().ok_or("Invalid subnet prefix")?;
    if prefix > u64::from(width) {
        return Err("Invalid subnet prefix".into());
    }
    let bits = width - prefix as u32;
    let mask = u128::MAX.checked_shr(128 - bits).unwrap_or(0);
    Ok((value & !mask, value | mask))
}
fn operation(request: &Value) -> Result<Value, String> {
    let family = request["family"].as_str().unwrap_or("");
    let parsed = address(&request["address"], family);
    if request["kind"] == "check" {
        return Ok(json!(parsed.is_ok_and(|(value, _, _)| request["rules"]
            .as_array()
            .is_some_and(|rules| rules
                .iter()
                .any(|rule| matches(rule, value))))));
    }
    let (value, text, width) = parsed?;
    let (low, high, label) = rule_bounds(request, family, value, &text, width)?;
    Ok(json!({"low":low.to_string(),"high":high.to_string(),"label":label}))
}
fn rule_bounds(
    request: &Value,
    family: &str,
    value: u128,
    text: &str,
    width: u32,
) -> Result<(u128, u128, String), String> {
    let family_label = if width == 32 { "IPv4" } else { "IPv6" };
    match request["kind"].as_str() {
        Some("address") => Ok((value, value, format!("Address: {family_label} {text}"))),
        Some("range") => {
            let (end, last, _) = address(&request["end"], family)?;
            if end < value {
                return Err("Range start must come before end".into());
            }
            Ok((value, end, format!("Range: {family_label} {text}-{last}")))
        }
        Some("subnet") => {
            let (low, high) = subnet(value, &request["prefix"], width)?;
            Ok((
                low,
                high,
                format!("Subnet: {family_label} {text}/{}", request["prefix"]),
            ))
        }
        _ => Err("Invalid BlockList operation".into()),
    }
}
pub fn native_blocklist(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let input = args.get(0).to_rust_string_lossy(scope);
    let result = serde_json::from_str(&input)
        .map_err(|e| e.to_string())
        .and_then(|request| operation(&request));
    let response = match result {
        Ok(value) => json!({"value":value}),
        Err(error) => json!({"error":error}),
    };
    if let Some(value) = v8::String::new(scope, &response.to_string()) {
        rv.set(value.into());
    }
}
