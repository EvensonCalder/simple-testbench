use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use anyhow::anyhow;

use crate::{
    cli::TestArgs,
    config::{LoadedConfig, SystemPrompt, resolve_selected_models},
    error::StbError,
    llm::{self, RetryPolicy},
    output::{self, ExecutionRecord, RecordStatus, ReportArtifacts, RunOutput},
    scoring,
};

#[derive(Debug, Clone)]
pub struct ExecutionSummary {
    pub output_dir: std::path::PathBuf,
    pub completed_requests: usize,
    pub failed_requests: usize,
    pub skipped_requests: usize,
    pub resumed_requests: usize,
    pub reports: ReportArtifacts,
}

#[derive(Debug)]
struct PendingSummary {
    completed_requests: usize,
    failed_requests: usize,
    skipped_requests: usize,
}

#[derive(Debug)]
struct ProviderPending<'a> {
    provider: &'a crate::config::ProviderConfig,
    concurrency: usize,
    requests: VecDeque<PendingRequest<'a>>,
}

impl<'a> ProviderPending<'a> {
    fn new(
        provider: &'a crate::config::ProviderConfig,
        concurrency_override: Option<usize>,
    ) -> Self {
        let concurrency = concurrency_override
            .map(|value| value.min(provider.concurrency as usize))
            .unwrap_or(provider.concurrency as usize)
            .max(1);

        Self {
            provider,
            concurrency,
            requests: VecDeque::new(),
        }
    }
}

#[derive(Debug)]
struct GlobalConcurrencyLimiter {
    max_in_flight: usize,
    in_flight: Mutex<usize>,
    available: Condvar,
}

impl GlobalConcurrencyLimiter {
    fn new(max_in_flight: usize) -> Self {
        Self {
            max_in_flight,
            in_flight: Mutex::new(0),
            available: Condvar::new(),
        }
    }

    fn acquire(self: &Arc<Self>) -> GlobalConcurrencyPermit {
        let mut in_flight = self
            .in_flight
            .lock()
            .expect("global concurrency mutex should not be poisoned");
        while *in_flight >= self.max_in_flight {
            in_flight = self
                .available
                .wait(in_flight)
                .expect("global concurrency mutex should not be poisoned");
        }

        *in_flight += 1;
        GlobalConcurrencyPermit {
            limiter: Arc::clone(self),
        }
    }
}

#[derive(Debug)]
struct GlobalConcurrencyPermit {
    limiter: Arc<GlobalConcurrencyLimiter>,
}

impl Drop for GlobalConcurrencyPermit {
    fn drop(&mut self) {
        let mut in_flight = self
            .limiter
            .in_flight
            .lock()
            .expect("global concurrency mutex should not be poisoned");
        *in_flight = in_flight.saturating_sub(1);
        self.limiter.available.notify_one();
    }
}

#[derive(Debug)]
struct PendingRequest<'a> {
    provider: &'a crate::config::ProviderConfig,
    model: &'a crate::config::ModelConfig,
    system_prompt: &'a SystemPrompt,
    test_case: &'a crate::config::TestCase,
    repeat_index: u32,
    record_key: output::RecordKey,
    model_key: (String, String),
    model_config_key: String,
    api_style: String,
}

#[derive(Debug)]
struct FailedExecution {
    error: String,
    attempts: u32,
    elapsed_ms: u64,
}

#[derive(Debug)]
enum WorkerOutcome {
    Completed(CompletedExecution),
    Failed(FailedExecution),
    Skipped(String),
}

#[derive(Debug)]
struct WorkerMessage<'a> {
    request: PendingRequest<'a>,
    outcome: WorkerOutcome,
}

#[derive(Debug)]
struct ProviderRateLimiter {
    interval: Duration,
    next_available: Mutex<Instant>,
}

impl ProviderRateLimiter {
    fn new(rpm: u32) -> Self {
        Self {
            interval: Duration::from_secs_f64(60.0 / f64::from(rpm)),
            next_available: Mutex::new(Instant::now()),
        }
    }

    fn wait_turn(&self) {
        loop {
            let sleep_for = {
                let mut next_available = self
                    .next_available
                    .lock()
                    .expect("rate limiter mutex should not be poisoned");
                let now = Instant::now();
                if *next_available <= now {
                    *next_available = now + self.interval;
                    return;
                }

                *next_available - now
            };

            thread::sleep(sleep_for);
        }
    }
}

pub fn run_test_session(
    loaded: &LoadedConfig,
    args: &TestArgs,
) -> anyhow::Result<ExecutionSummary> {
    if loaded.tests.is_empty() {
        return Err(
            StbError::InvalidConfig("no tests are configured for execution".to_string()).into(),
        );
    }

    let selected =
        resolve_selected_models(loaded, args.provider.as_deref(), args.model.as_deref())?;
    let output_dir = output::prepare_output_dir(args.output_dir.as_deref(), args.fresh)?;
    let output_path = output::output_json_path(&output_dir);
    let system_prompt_index = system_prompt_index(loaded);
    let scoring_config = scoring::load_scoring_config(&args.input, args.score_archive.as_deref())?;
    let retry_policy = RetryPolicy::from_retry_count(args.retry);
    let mut run_output = if args.fresh || !output_path.exists() {
        RunOutput::new()
    } else {
        output::load_run_output(&output_path)?
    };
    let mut existing_records = run_output
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.key(), index))
        .collect::<HashMap<_, _>>();
    let mut disabled_models = disabled_models(&run_output);
    let mut pending_by_provider = HashMap::<String, ProviderPending<'_>>::new();
    let mut completed_requests = 0usize;
    let mut failed_requests = 0usize;
    let mut skipped_requests = 0usize;
    let mut resumed_requests = 0usize;

    for resolved in selected {
        let model_instance_id = resolved.model.instance_id();
        let model_config_key = resolved.model.config_key();
        let api_style = resolved.model.api_style.as_str().to_string();
        let model_key = (
            resolved.provider.provider_id.clone(),
            model_instance_id.clone(),
        );
        for test_case in &loaded.tests {
            let system_prompt = system_prompt_index
                .get(test_case.system_prompt.as_str())
                .copied()
                .ok_or_else(|| {
                    anyhow!(
                        "missing system prompt {} for test {}",
                        test_case.system_prompt,
                        test_case.id
                    )
                })?;

            let repeat_count = args.repeat.unwrap_or(test_case.repeat);
            for repeat_index in 1..=repeat_count {
                let record_key = output::RecordKey {
                    provider_id: resolved.provider.provider_id.clone(),
                    model_id: resolved.model.model_id.clone(),
                    model_instance_id: model_instance_id.clone(),
                    test_id: test_case.id.clone(),
                    repeat_index,
                };

                if let Some(index) = existing_records.get(&record_key).copied() {
                    if maybe_complete_existing_record(
                        &mut run_output.records[index],
                        loaded,
                        test_case,
                        &scoring_config,
                        &retry_policy,
                        args.disable_post_process,
                        args.verbose,
                    )? {
                        output::write_run_output(&output_path, &run_output)?;
                    }
                    resumed_requests += 1;
                    continue;
                }

                if let Some(reason) = disabled_models.get(&model_key).cloned() {
                    let record = ExecutionRecord {
                        id: output::next_record_id(),
                        provider_id: resolved.provider.provider_id.clone(),
                        model_id: resolved.model.model_id.clone(),
                        model_instance_id: model_instance_id.clone(),
                        model_config_key: model_config_key.clone(),
                        test_id: test_case.id.clone(),
                        repeat_index,
                        api_style: api_style.clone(),
                        status: RecordStatus::SkippedModelDisabled,
                        attempts: 0,
                        elapsed_ms: 0,
                        output_text: None,
                        processed_output: None,
                        post_process_applied: false,
                        post_process_retries: 0,
                        scores: Vec::new(),
                        error: Some(reason.clone()),
                    };
                    let record_index = run_output.records.len();
                    run_output.records.push(record);
                    existing_records.insert(record_key, record_index);
                    output::write_run_output(&output_path, &run_output)?;
                    skipped_requests += 1;
                    continue;
                }

                pending_by_provider
                    .entry(resolved.provider.provider_id.clone())
                    .or_insert_with(|| ProviderPending::new(resolved.provider, args.concurrency))
                    .requests
                    .push_back(PendingRequest {
                        provider: resolved.provider,
                        model: resolved.model,
                        system_prompt,
                        test_case,
                        repeat_index,
                        record_key,
                        model_key: model_key.clone(),
                        model_config_key: model_config_key.clone(),
                        api_style: api_style.clone(),
                    });
            }
        }
    }

    let pending_summary = run_pending_requests(
        loaded,
        pending_by_provider,
        &retry_policy,
        &scoring_config,
        args,
        &output_path,
        &mut run_output,
        &mut existing_records,
        &mut disabled_models,
    )?;
    completed_requests += pending_summary.completed_requests;
    failed_requests += pending_summary.failed_requests;
    skipped_requests += pending_summary.skipped_requests;

    let reports = output::write_reports(&output_dir, &run_output, args.json)?;

    Ok(ExecutionSummary {
        output_dir,
        completed_requests,
        failed_requests,
        skipped_requests,
        resumed_requests,
        reports,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_pending_requests<'a>(
    loaded: &'a LoadedConfig,
    pending_by_provider: HashMap<String, ProviderPending<'a>>,
    retry_policy: &'a RetryPolicy,
    scoring_config: &'a scoring::LoadedScoringConfig,
    args: &'a TestArgs,
    output_path: &std::path::Path,
    run_output: &mut RunOutput,
    existing_records: &mut HashMap<output::RecordKey, usize>,
    disabled_models: &mut HashMap<(String, String), String>,
) -> anyhow::Result<PendingSummary> {
    if pending_by_provider.is_empty() {
        return Ok(PendingSummary {
            completed_requests: 0,
            failed_requests: 0,
            skipped_requests: 0,
        });
    }

    let disabled_state = Arc::new(Mutex::new(disabled_models.clone()));
    let global_limiter = args
        .concurrency
        .map(|concurrency| Arc::new(GlobalConcurrencyLimiter::new(concurrency)));
    let (tx, rx) = mpsc::channel::<WorkerMessage<'a>>();

    let mut summary = PendingSummary {
        completed_requests: 0,
        failed_requests: 0,
        skipped_requests: 0,
    };

    thread::scope(|scope| {
        for provider_pending in pending_by_provider.into_values() {
            let request_queue = Arc::new(Mutex::new(provider_pending.requests));
            let limiter = Arc::new(ProviderRateLimiter::new(provider_pending.provider.rpm));

            for _ in 0..provider_pending.concurrency {
                let request_queue = Arc::clone(&request_queue);
                let limiter = Arc::clone(&limiter);
                let tx = tx.clone();
                let disabled_state = Arc::clone(&disabled_state);
                let global_limiter = global_limiter.clone();

                scope.spawn(move || {
                    loop {
                        let Some(request) = pop_next_request(&request_queue) else {
                            break;
                        };

                        if let Some(reason) = disabled_state
                            .lock()
                            .expect("disabled model mutex should not be poisoned")
                            .get(&request.model_key)
                            .cloned()
                        {
                            let _ = tx.send(WorkerMessage {
                                request,
                                outcome: WorkerOutcome::Skipped(reason),
                            });
                            continue;
                        }

                        limiter.wait_turn();
                        let _global_permit =
                            global_limiter.as_ref().map(|limiter| limiter.acquire());
                        let request_started = Instant::now();
                        let outcome = match execute_request_with_scoring(
                            loaded,
                            request.provider,
                            request.model,
                            request.system_prompt,
                            request.test_case,
                            retry_policy,
                            scoring_config,
                            args.disable_post_process,
                            args.verbose,
                        ) {
                            Ok(execution) => WorkerOutcome::Completed(execution),
                            Err(error) => {
                                let rendered = format!("{error:#}");
                                disabled_state
                                    .lock()
                                    .expect("disabled model mutex should not be poisoned")
                                    .insert(request.model_key.clone(), rendered.clone());
                                WorkerOutcome::Failed(FailedExecution {
                                    error: rendered,
                                    attempts: u32::from(args.retry) + 1,
                                    elapsed_ms: request_started.elapsed().as_millis() as u64,
                                })
                            }
                        };

                        if tx.send(WorkerMessage { request, outcome }).is_err() {
                            break;
                        }
                    }
                });
            }
        }

        drop(tx);

        for message in rx {
            match message.outcome {
                WorkerOutcome::Completed(execution) => {
                    push_completed_record(run_output, existing_records, message.request, execution);
                    output::write_run_output(output_path, run_output)?;
                    summary.completed_requests += 1;
                }
                WorkerOutcome::Failed(failure) => {
                    if args.verbose {
                        println!(
                            "request failed for {}/{} test={} error={}",
                            message.request.provider.provider_id,
                            message.request.model.model_id,
                            message.request.test_case.id,
                            failure.error
                        );
                    }

                    push_failed_record(run_output, existing_records, message.request, failure);
                    output::write_run_output(output_path, run_output)?;
                    summary.failed_requests += 1;
                }
                WorkerOutcome::Skipped(reason) => {
                    push_skipped_record(run_output, existing_records, message.request, reason);
                    output::write_run_output(output_path, run_output)?;
                    summary.skipped_requests += 1;
                }
            }
        }

        Ok::<(), anyhow::Error>(())
    })?;

    *disabled_models = disabled_state
        .lock()
        .expect("disabled model mutex should not be poisoned")
        .clone();

    Ok(summary)
}

fn pop_next_request<'a>(
    request_queue: &Arc<Mutex<VecDeque<PendingRequest<'a>>>>,
) -> Option<PendingRequest<'a>> {
    request_queue
        .lock()
        .expect("request queue mutex should not be poisoned")
        .pop_front()
}

fn push_completed_record(
    run_output: &mut RunOutput,
    existing_records: &mut HashMap<output::RecordKey, usize>,
    request: PendingRequest<'_>,
    execution: CompletedExecution,
) {
    let record_key = request.record_key.clone();
    let record = ExecutionRecord {
        id: output::next_record_id(),
        provider_id: request.provider.provider_id.clone(),
        model_id: request.model.model_id.clone(),
        model_instance_id: request.record_key.model_instance_id.clone(),
        model_config_key: request.model_config_key,
        test_id: request.test_case.id.clone(),
        repeat_index: request.repeat_index,
        api_style: request.api_style,
        status: RecordStatus::Success,
        attempts: execution.attempts,
        elapsed_ms: execution.elapsed_ms,
        output_text: Some(execution.output_text),
        processed_output: Some(execution.processed_output),
        post_process_applied: execution.post_process_applied,
        post_process_retries: execution.post_process_retries,
        scores: execution.scores,
        error: None,
    };
    let record_index = run_output.records.len();
    run_output.records.push(record);
    existing_records.insert(record_key, record_index);
}

fn push_failed_record(
    run_output: &mut RunOutput,
    existing_records: &mut HashMap<output::RecordKey, usize>,
    request: PendingRequest<'_>,
    failure: FailedExecution,
) {
    let record_key = request.record_key.clone();
    let record = ExecutionRecord {
        id: output::next_record_id(),
        provider_id: request.provider.provider_id.clone(),
        model_id: request.model.model_id.clone(),
        model_instance_id: request.record_key.model_instance_id.clone(),
        model_config_key: request.model_config_key,
        test_id: request.test_case.id.clone(),
        repeat_index: request.repeat_index,
        api_style: request.api_style,
        status: RecordStatus::Failed,
        attempts: failure.attempts,
        elapsed_ms: failure.elapsed_ms,
        output_text: None,
        processed_output: None,
        post_process_applied: false,
        post_process_retries: 0,
        scores: Vec::new(),
        error: Some(failure.error),
    };
    let record_index = run_output.records.len();
    run_output.records.push(record);
    existing_records.insert(record_key, record_index);
}

fn push_skipped_record(
    run_output: &mut RunOutput,
    existing_records: &mut HashMap<output::RecordKey, usize>,
    request: PendingRequest<'_>,
    reason: String,
) {
    let record_key = request.record_key.clone();
    let record = ExecutionRecord {
        id: output::next_record_id(),
        provider_id: request.provider.provider_id.clone(),
        model_id: request.model.model_id.clone(),
        model_instance_id: request.record_key.model_instance_id.clone(),
        model_config_key: request.model_config_key,
        test_id: request.test_case.id.clone(),
        repeat_index: request.repeat_index,
        api_style: request.api_style,
        status: RecordStatus::SkippedModelDisabled,
        attempts: 0,
        elapsed_ms: 0,
        output_text: None,
        processed_output: None,
        post_process_applied: false,
        post_process_retries: 0,
        scores: Vec::new(),
        error: Some(reason),
    };
    let record_index = run_output.records.len();
    run_output.records.push(record);
    existing_records.insert(record_key, record_index);
}

#[derive(Debug)]
struct CompletedExecution {
    output_text: String,
    processed_output: String,
    attempts: u32,
    elapsed_ms: u64,
    post_process_applied: bool,
    post_process_retries: u32,
    scores: Vec<crate::output::ScoreResult>,
}

#[allow(clippy::too_many_arguments)]
fn execute_request_with_scoring(
    loaded: &LoadedConfig,
    provider: &crate::config::ProviderConfig,
    model: &crate::config::ModelConfig,
    system_prompt: &SystemPrompt,
    test_case: &crate::config::TestCase,
    retry_policy: &RetryPolicy,
    scoring_config: &scoring::LoadedScoringConfig,
    disable_post_process: bool,
    verbose: bool,
) -> anyhow::Result<CompletedExecution> {
    let mut total_attempts = 0u32;
    let mut total_elapsed_ms = 0u64;
    let mut post_process_retries = 0u32;
    let request_label = format!("test={}", test_case.id);
    let user_prompt = llm::build_test_user_prompt(test_case)?;

    loop {
        let execution = llm::execute_model_request(
            provider,
            model,
            &system_prompt.text,
            &user_prompt,
            retry_policy,
            verbose,
            &request_label,
        )?;
        total_attempts += u32::from(execution.attempts);
        total_elapsed_ms += execution.elapsed_ms;

        let (processed_output, post_process_applied, should_retry, max_retry) =
            post_process_output(&execution.output_text, scoring_config, disable_post_process)?;

        if should_retry && post_process_retries < max_retry {
            post_process_retries += 1;

            if verbose {
                println!(
                    "post-process requested retry for {}/{} test={} retry={}/{}",
                    provider.provider_id,
                    model.model_id,
                    test_case.id,
                    post_process_retries,
                    max_retry,
                );
            }

            continue;
        }

        let scores = scoring::score_processed_output(
            scoring_config,
            loaded,
            test_case,
            &processed_output,
            retry_policy,
            verbose,
        );

        return Ok(CompletedExecution {
            output_text: execution.output_text,
            processed_output,
            attempts: total_attempts,
            elapsed_ms: total_elapsed_ms,
            post_process_applied,
            post_process_retries,
            scores,
        });
    }
}

fn maybe_complete_existing_record(
    record: &mut ExecutionRecord,
    loaded: &LoadedConfig,
    test_case: &crate::config::TestCase,
    scoring_config: &scoring::LoadedScoringConfig,
    retry_policy: &RetryPolicy,
    disable_post_process: bool,
    verbose: bool,
) -> anyhow::Result<bool> {
    if record.status != RecordStatus::Success {
        return Ok(false);
    }

    let Some(raw_output) = record.output_text.as_deref() else {
        return Ok(false);
    };

    let mut changed = false;

    if record.processed_output.is_none() {
        let (processed_output, post_process_applied, _, _) =
            post_process_output(raw_output, scoring_config, disable_post_process)?;
        record.processed_output = Some(processed_output);
        record.post_process_applied = post_process_applied;
        record.post_process_retries = 0;
        changed = true;
    }

    if !has_all_scores(record, scoring_config) {
        let processed_output = record.processed_output.as_deref().unwrap_or(raw_output);

        for score in scoring::score_processed_output(
            scoring_config,
            loaded,
            test_case,
            processed_output,
            retry_policy,
            verbose,
        ) {
            if record
                .scores
                .iter()
                .any(|existing| existing.name == score.name)
            {
                continue;
            }

            record.scores.push(score);
            changed = true;
        }
    }

    Ok(changed)
}

fn post_process_output(
    raw_output: &str,
    scoring_config: &scoring::LoadedScoringConfig,
    disable_post_process: bool,
) -> anyhow::Result<(String, bool, bool, u32)> {
    if disable_post_process {
        return Ok((raw_output.to_string(), false, false, 0));
    }

    let Some(script) = scoring_config.post_process.as_deref() else {
        return Ok((raw_output.to_string(), false, false, 0));
    };

    let outcome = scoring::apply_post_process(script, raw_output)?;
    Ok((outcome.output, true, outcome.retry, outcome.max_retry))
}

fn has_all_scores(record: &ExecutionRecord, scoring_config: &scoring::LoadedScoringConfig) -> bool {
    if scoring_config.scorers.is_empty() {
        return true;
    }

    let existing = record
        .scores
        .iter()
        .map(|score| score.name.as_str())
        .collect::<HashSet<_>>();

    scoring_config
        .scorer_names()
        .into_iter()
        .all(|name| existing.contains(name))
}

fn system_prompt_index(loaded: &LoadedConfig) -> HashMap<&str, &SystemPrompt> {
    loaded
        .system_prompts
        .iter()
        .map(|prompt| (prompt.id.as_str(), prompt))
        .collect()
}

fn disabled_models(run_output: &RunOutput) -> HashMap<(String, String), String> {
    run_output
        .records
        .iter()
        .filter(|record| record.status == RecordStatus::Failed)
        .filter_map(|record| {
            record.error.as_ref().map(|error| {
                (
                    (record.provider_id.clone(), record.model_instance_id.clone()),
                    error.clone(),
                )
            })
        })
        .collect()
}
