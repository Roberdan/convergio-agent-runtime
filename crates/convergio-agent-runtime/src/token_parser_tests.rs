#![allow(dead_code)]

use super::*;

#[test]
fn parse_claude_json_output() {
    let log = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":5140,"duration_api_ms":5089,"num_turns":1,"result":"test","stop_reason":"end_turn","session_id":"abc","total_cost_usd":0.076,"usage":{"input_tokens":100,"cache_creation_input_tokens":11000,"cache_read_input_tokens":12000,"output_tokens":50},"modelUsage":{"claude-opus-4-6[1m]":{"inputTokens":100,"outputTokens":50,"costUSD":0.076}}}"#;
    let usage = parse_agent_log(log).expect("should parse");
    assert_eq!(usage.backend, "claude");
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.output_tokens, 50);
    assert_eq!(usage.cache_read_tokens, 12000);
    assert_eq!(usage.cache_creation_tokens, 11000);
    assert!((usage.cost_usd - 0.076).abs() < 0.001);
    assert_eq!(usage.num_turns, 1);
    assert_eq!(usage.duration_ms, 5140);
    assert!(usage.model.contains("claude-opus"));
}

#[test]
fn parse_copilot_jsonl_output() {
    let log = r#"{"type":"session.tools_updated","data":{}}
{"type":"user.message","data":{"content":"test"}}
{"type":"assistant.message","data":{"messageId":"a","content":"hello","outputTokens":20}}
{"type":"assistant.message","data":{"messageId":"b","content":"world","outputTokens":30}}
{"type":"result","timestamp":"2026-04-10","sessionId":"x","exitCode":0,"usage":{"premiumRequests":6,"totalApiDurationMs":3000,"sessionDurationMs":8000}}"#;
    let usage = parse_agent_log(log).expect("should parse");
    assert_eq!(usage.backend, "copilot");
    assert_eq!(usage.output_tokens, 50);
    assert_eq!(usage.num_turns, 2);
    assert_eq!(usage.duration_ms, 8000);
    assert!(usage.cost_usd > 0.0);
    // Input tokens estimated from premiumRequests
    assert!(usage.input_tokens > 0);
}

#[test]
fn parse_empty_log() {
    assert!(parse_agent_log("").is_none());
    assert!(parse_agent_log("not json").is_none());
}

#[test]
fn parse_copilot_no_messages() {
    let log = r#"{"type":"result","usage":{"premiumRequests":0,"sessionDurationMs":100}}"#;
    assert!(parse_agent_log(log).is_none());
}
