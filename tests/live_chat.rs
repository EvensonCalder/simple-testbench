use std::{fs, path::Path};

use assert_cmd::Command;
use httpmock::{Method::POST, MockServer};
use predicates::prelude::*;
use tempfile::tempdir;

fn write_basic_suite(input_dir: &Path, prompt_text: &str, input_text: &str) {
    fs::write(
        input_dir.join("system_prompts.json"),
        format!(
            r#"{{
  "system_prompts": [
    {{
      "id": "todo-json",
      "text": "{}"
    }}
  ]
}}"#,
            prompt_text
        ),
    )
    .expect("system prompts should be written");

    fs::write(
        input_dir.join("tests.json"),
        format!(
            r#"{{
  "tests": [
    {{
      "id": "todo-1",
      "system_prompt": "todo-json",
      "input": [
        {{
          "type": "text",
          "text": "{}"
        }}
      ],
      "repeat": 1
    }}
  ]
}}"#,
            input_text
        ),
    )
    .expect("tests should be written");
}

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

#[test]
fn resumes_from_existing_output_without_repeating_requests() {
    let server = MockServer::start();
    let temp = tempdir().expect("temp dir should exist");
    let input_dir = temp.path().join("input");
    let output_dir = temp.path().join("out");
    fs::create_dir(&input_dir).expect("input dir should be created");

    let mock = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"choices":[{"message":{"content":"ok"}}]}"#);
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
      "api_style": "openai_chat_completions"
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
      "text": "Return text only."
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
          "text": "Hello"
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
        .stdout(predicate::str::contains("resumed requests: 0"));

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
        .stdout(predicate::str::contains("completed requests: 0"))
        .stdout(predicate::str::contains("resumed requests: 1"));

    assert_eq!(mock.hits(), 1);
}

#[test]
fn executes_scoring_pipeline_and_persists_processed_scores() {
    let server = MockServer::start();
    let temp = tempdir().expect("temp dir should exist");
    let input_dir = temp.path().join("input");
    let output_dir = temp.path().join("out");
    fs::create_dir(&input_dir).expect("input dir should be created");

    server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"choices":[{"message":{"content":"  clean me  "}}]}"#);
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
      "api_style": "openai_chat_completions"
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
      "text": "Return text only."
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
          "text": "Hello"
        }
      ],
      "repeat": 1
    }
  ]
}"#,
    )
    .expect("tests should be written");

    fs::write(
        input_dir.join("post_process.lua"),
        r#"return function(raw_output)
  return {
    output = tostring(raw_output):gsub("^%s+", ""):gsub("%s+$", "")
  }
end"#,
    )
    .expect("post-process should be written");

    fs::write(
        input_dir.join("scoring.json"),
        r#"{
  "scoring": [
    {
      "name": "trimmed",
      "kind": "lua",
      "file": "score_trimmed.lua"
    }
  ]
}"#,
    )
    .expect("scoring should be written");

    fs::write(
        input_dir.join("score_trimmed.lua"),
        r#"return function(output)
  if output == "clean me" then
    return 100
  end

  return 0
end"#,
    )
    .expect("lua scorer should be written");

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
    assert!(output_json.contains("\"processed_output\": \"clean me\""));
    assert!(output_json.contains("\"post_process_applied\": true"));
    assert!(output_json.contains("\"name\": \"trimmed\""));
    assert!(output_json.contains("\"score\": 100"));
}

#[test]
fn executes_openai_responses_and_writes_output() {
    let server = MockServer::start();
    let temp = tempdir().expect("temp dir should exist");
    let input_dir = temp.path().join("input");
    let output_dir = temp.path().join("out");
    fs::create_dir(&input_dir).expect("input dir should be created");

    server.mock(|when, then| {
        when.method(POST).path("/responses");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"output":[{"type":"reasoning","summary":"hidden"},{"type":"message","content":[{"type":"output_text","text":"final responses output"}]}]}"#);
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
        "openai_responses": "{}/responses"
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
      "model_id": "responses-model",
      "api_style": "openai_responses"
    }
  ]
}"#,
    )
    .expect("models should be written");

    write_basic_suite(&input_dir, "Return text only.", "Hello");

    Command::cargo_bin("stb")
        .expect("binary should build")
        .args(["test", "-i"])
        .arg(&input_dir)
        .args([
            "--provider",
            "mock",
            "--model",
            "responses-model",
            "--output-dir",
        ])
        .arg(&output_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("completed requests: 1"));

    let output_json = fs::read_to_string(output_dir.join("output.json"))
        .expect("output.json should exist after execution");
    assert!(output_json.contains("responses-model"));
    assert!(output_json.contains("final responses output"));
}

#[test]
fn executes_anthropic_messages_and_discards_thinking() {
    let server = MockServer::start();
    let temp = tempdir().expect("temp dir should exist");
    let input_dir = temp.path().join("input");
    let output_dir = temp.path().join("out");
    fs::create_dir(&input_dir).expect("input dir should be created");

    server.mock(|when, then| {
        when.method(POST).path("/messages");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"content":[{"type":"thinking","thinking":"hidden thoughts"},{"type":"text","text":"final anthropic output"}]}"#);
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
        "anthropic_messages": "{}/messages"
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
      "model_id": "messages-model",
      "api_style": "anthropic_messages"
    }
  ]
}"#,
    )
    .expect("models should be written");

    write_basic_suite(&input_dir, "Return text only.", "Hello");

    Command::cargo_bin("stb")
        .expect("binary should build")
        .args(["test", "-i"])
        .arg(&input_dir)
        .args([
            "--provider",
            "mock",
            "--model",
            "messages-model",
            "--output-dir",
        ])
        .arg(&output_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("completed requests: 1"));

    let output_json = fs::read_to_string(output_dir.join("output.json"))
        .expect("output.json should exist after execution");
    assert!(output_json.contains("final anthropic output"));
    assert!(!output_json.contains("hidden thoughts"));
}

#[test]
fn executes_ai_scoring_and_writes_reports() {
    let server = MockServer::start();
    let temp = tempdir().expect("temp dir should exist");
    let input_dir = temp.path().join("input");
    let output_dir = temp.path().join("out");
    fs::create_dir(&input_dir).expect("input dir should be created");

    server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"choices":[{"message":{"content":"candidate answer"}}]}"#);
    });
    server.mock(|when, then| {
        when.method(POST).path("/responses");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"{\"score\":87,\"reason\":\"pretty good\"}"}]}]}"#);
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
        "openai_chat_completions": "{}",
        "openai_responses": "{}/responses"
      }}
    }}
  ]
}}"#,
            server.base_url(),
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
      "api_style": "openai_chat_completions"
    }
  ]
}"#,
    )
    .expect("models should be written");

    write_basic_suite(&input_dir, "Return text only.", "Hello");

    fs::write(
        input_dir.join("scoring.json"),
        r#"{
  "scoring": [
    {
      "name": "judge",
      "kind": "ai",
      "file": "judge.json"
    }
  ]
}"#,
    )
    .expect("scoring should be written");
    fs::write(
        input_dir.join("judge.json"),
        r#"{
  "provider_id": "mock",
  "model_id": "judge-model",
  "api_style": "openai_responses",
  "temperature": 0,
  "system_prompt": "Score the candidate output."
}"#,
    )
    .expect("judge config should be written");

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
        .args(["--json", "--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"score_name\": \"judge\""));

    let output_json = fs::read_to_string(output_dir.join("output.json"))
        .expect("output.json should exist after execution");
    assert!(output_json.contains("\"kind\": \"ai\""));
    assert!(output_json.contains("\"score\": 87"));
    assert!(output_json.contains("pretty good"));

    let results_json = fs::read_to_string(output_dir.join("results.json"))
        .expect("results.json should exist after execution");
    assert!(results_json.contains("\"score_name\": \"judge\""));
    assert!(results_json.contains("\"mean\": 87.0"));

    let score_mean_csv = fs::read_to_string(output_dir.join("score_mean.csv"))
        .expect("score_mean.csv should exist after execution");
    let score_std_csv = fs::read_to_string(output_dir.join("score_std.csv"))
        .expect("score_std.csv should exist after execution");
    assert!(score_mean_csv.contains("judge"));
    assert!(score_mean_csv.contains("87.0000"));
    assert!(score_std_csv.contains("0.0000"));
}

#[test]
fn resumes_and_backfills_scores_without_repeating_benchmark_requests() {
    let server = MockServer::start();
    let temp = tempdir().expect("temp dir should exist");
    let input_dir = temp.path().join("input");
    let output_dir = temp.path().join("out");
    fs::create_dir(&input_dir).expect("input dir should be created");

    let chat_mock = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .body(r#"{"choices":[{"message":{"content":"candidate answer"}}]}"#);
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
      "api_style": "openai_chat_completions"
    }
  ]
}"#,
    )
    .expect("models should be written");
    write_basic_suite(&input_dir, "Return text only.", "Hello");

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
        .stdout(predicate::str::contains("completed requests: 1"));

    fs::write(
        input_dir.join("scoring.json"),
        r#"{
  "scoring": [
    {
      "name": "judge",
      "kind": "lua",
      "file": "judge.lua"
    }
  ]
}"#,
    )
    .expect("scoring should be written");
    fs::write(
        input_dir.join("judge.lua"),
        r#"return function(output)
  if output == "candidate answer" then
    return 100
  end

  return 0
end"#,
    )
    .expect("judge lua should be written");

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
        .stdout(predicate::str::contains("completed requests: 0"))
        .stdout(predicate::str::contains("resumed requests: 1"));

    assert_eq!(chat_mock.hits(), 1);

    let output_json = fs::read_to_string(output_dir.join("output.json"))
        .expect("output.json should exist after execution");
    assert!(output_json.contains("\"name\": \"judge\""));
    assert!(output_json.contains("\"score\": 100"));

    let score_mean_csv = fs::read_to_string(output_dir.join("score_mean.csv"))
        .expect("score_mean.csv should exist after execution");
    assert!(score_mean_csv.contains("100.0000"));
}
