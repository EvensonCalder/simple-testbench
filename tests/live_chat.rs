use std::fs;

use assert_cmd::Command;
use httpmock::{Method::POST, MockServer};
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn executes_openai_chat_completion_and_writes_output() {
    let server = MockServer::start();
    let temp = tempdir().expect("temp dir should exist");
    let input_dir = temp.path().join("input");
    let output_dir = temp.path().join("out");
    fs::create_dir(&input_dir).expect("input dir should be created");

    server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"choices":[{"message":{"content":"{\"todo\":\"buy milk\",\"time\":null,\"location\":\"supermarket\"}"}}]}"#);
    });

    fs::write(
        input_dir.join("providers.json"),
        format!(
            r#"{{
  "providers": [
    {{
      "provider_id": "mock",
      "key": "test-key",
      "env_key": null,
      "concurrency": 1,
      "rpm": 10,
      "endpoints": {{
        "openai_chat_completions": "{}"
      }}
    }}
  ]
}}"#,
            server.base_url()
        ),
    )
    .expect("providers should be written");

    fs::write(
        input_dir.join("models.json"),
        r#"{
  "models": [
    {
      "provider_id": "mock",
      "model_id": "chat-model",
      "api_style": "openai_chat_completions",
      "temperature": 0,
      "max_output_tokens": 128
    }
  ]
}"#,
    )
    .expect("models should be written");

    fs::write(
        input_dir.join("system_prompts.json"),
        r#"{
  "system_prompts": [
    {
      "id": "todo-json",
      "text": "Return JSON only."
    }
  ]
}"#,
    )
    .expect("system prompts should be written");

    fs::write(
        input_dir.join("tests.json"),
        r#"{
  "tests": [
    {
      "id": "todo-1",
      "system_prompt": "todo-json",
      "input": [
        {
          "type": "text",
          "text": "Buy milk at the supermarket"
        }
      ],
      "repeat": 1
    }
  ]
}"#,
    )
    .expect("tests should be written");

    Command::cargo_bin("stb")
        .expect("binary should build")
        .args(["test", "-i"])
        .arg(&input_dir)
        .args([
            "--provider",
            "mock",
            "--model",
            "chat-model",
            "--output-dir",
        ])
        .arg(&output_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("completed requests: 1"))
        .stdout(predicate::str::contains("failed requests: 0"));

    let output_json = fs::read_to_string(output_dir.join("output.json"))
        .expect("output.json should exist after execution");
    assert!(output_json.contains("chat-model"));
    assert!(output_json.contains("buy milk"));
}
