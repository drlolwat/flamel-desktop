use super::*;
#[path = "shell/live.rs"]
mod live;

pub(super) fn handle(
    name: &str,
    args: &Value,
    base_url: &str,
    token: &str,
    _session_id: &str,
    project_path: &str,
) -> Result<String, String> {
    match name {
        "shell" | "flamel_shell" | "shell_run" | "shell_start" | "shell_read" | "shell_write"
        | "shell_kill" | "shell_status" | "multi_shell" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .or_else(|| name.strip_prefix("shell_"))
                .unwrap_or(if name == "multi_shell" {
                    "multi"
                } else {
                    "run"
                });

            match action {
                "run" => {
                    let command = args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .ok_or("command required")?
                        .to_string();
                    let cwd = args
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .unwrap_or(project_path)
                        .to_string();
                    let timeout_secs = args
                        .get("timeout_secs")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(300);

                    validate_command(&command)?;

                    if let Some(api) = live::PtyApi::for_host(base_url, token)? {
                        let started = api.start(&command, &cwd)?;
                        let command_id = started["command_session_id"]
                            .as_str()
                            .ok_or("Missing command session id")?;
                        return match api.forward(command_id, Some(timeout_secs)) {
                            Ok(output) => Ok(output),
                            Err(error) => {
                                let id = started["terminal_session_id"]
                                    .as_str()
                                    .ok_or("Missing terminal id")?;
                                api.call(
                                    reqwest::Method::DELETE,
                                    &format!("/api/terminal/{id}"),
                                    None,
                                )
                                .map_err(|cleanup| {
                                    format!(
                                        "{error}; could not confirm terminal shutdown: {cleanup}"
                                    )
                                })?;
                                Err(format!(
                                    "Terminal stopped after a timeout or lost stream: {error}"
                                ))
                            }
                        };
                    }

                    // Streaming execution — all shell commands stream output via MCP log notifications
                    // and typed command-event notifications for the unified console.
                    let command_event = start_shell_command_event(None, &command, &cwd);
                    let auth_env = github_cli_auth_env(base_url, token);
                    let mut cmd = flamel::mcp_parent_guard::command("sh");
                    cmd.arg("-c")
                        .arg(&command)
                        .current_dir(&cwd)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped());
                    inject_shell_auth_env(&mut cmd, &auth_env);
                    configure_shell_child(&mut cmd);
                    let mut child = match cmd.spawn() {
                        Ok(child) => child,
                        Err(e) => {
                            let error = format!("Failed to spawn: {e}");
                            emit_shell_error_event(&command_event, &error);
                            finish_and_close_shell_command_event(&command_event, "failed", None);
                            return Err(error);
                        }
                    };

                    let (tx, rx) = std::sync::mpsc::channel::<String>();
                    if let Some(stdout_pipe) = child.stdout.take() {
                        let tx1 = tx.clone();
                        std::thread::spawn(move || {
                            let reader = io::BufReader::new(stdout_pipe);
                            for line in io::BufRead::lines(reader) {
                                match line {
                                    Ok(l) => {
                                        let _ = tx1.send(l);
                                    }
                                    Err(_) => break,
                                }
                            }
                        });
                    }
                    if let Some(stderr_pipe) = child.stderr.take() {
                        let tx2 = tx.clone();
                        std::thread::spawn(move || {
                            let reader = io::BufReader::new(stderr_pipe);
                            for line in io::BufRead::lines(reader) {
                                match line {
                                    Ok(l) => {
                                        let _ = tx2.send(format!("[stderr] {l}"));
                                    }
                                    Err(_) => break,
                                }
                            }
                        });
                    }
                    drop(tx);

                    let start = std::time::Instant::now();
                    let mut all_output = String::new();
                    let mut live_log_chars = 0usize;
                    let mut live_log_omitted = 0usize;
                    let mut exit_status = None;
                    loop {
                        match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                            Ok(line) => {
                                emit_shell_output_line(&command_event, &line);
                                send_shell_run_log(
                                    &line,
                                    &mut live_log_chars,
                                    &mut live_log_omitted,
                                );
                                append_capped_shell_output(&mut all_output, &line);
                                if start.elapsed().as_secs() >= timeout_secs {
                                    terminate_shell_child(&mut child);
                                    drain_shell_output_for(
                                        &rx,
                                        &mut all_output,
                                        std::time::Duration::from_millis(
                                            SHELL_RUN_TIMEOUT_DRAIN_MS,
                                        ),
                                        &mut live_log_chars,
                                        &mut live_log_omitted,
                                        Some(&command_event),
                                    );
                                    send_shell_run_omitted_notice(&mut live_log_omitted);
                                    emit_shell_error_event(
                                        &command_event,
                                        &format!(
                                            "Command timed out after {timeout_secs}s; process group terminated."
                                        ),
                                    );
                                    finish_and_close_shell_command_event(
                                        &command_event,
                                        "timed_out",
                                        None,
                                    );
                                    return Err(format!(
                                        "Command timed out after {timeout_secs}s; process group terminated.\n\nOutput so far:\n{all_output}"
                                    ));
                                }
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                if let Ok(Some(status)) = child.try_wait() {
                                    exit_status = Some(status);
                                    drain_shell_output_for(
                                        &rx,
                                        &mut all_output,
                                        std::time::Duration::from_millis(250),
                                        &mut live_log_chars,
                                        &mut live_log_omitted,
                                        Some(&command_event),
                                    );
                                    send_shell_run_omitted_notice(&mut live_log_omitted);
                                    break;
                                }
                                if start.elapsed().as_secs() >= timeout_secs {
                                    terminate_shell_child(&mut child);
                                    drain_shell_output_for(
                                        &rx,
                                        &mut all_output,
                                        std::time::Duration::from_millis(
                                            SHELL_RUN_TIMEOUT_DRAIN_MS,
                                        ),
                                        &mut live_log_chars,
                                        &mut live_log_omitted,
                                        Some(&command_event),
                                    );
                                    send_shell_run_omitted_notice(&mut live_log_omitted);
                                    emit_shell_error_event(
                                        &command_event,
                                        &format!(
                                            "Command timed out after {timeout_secs}s; process group terminated."
                                        ),
                                    );
                                    finish_and_close_shell_command_event(
                                        &command_event,
                                        "timed_out",
                                        None,
                                    );
                                    return Err(format!(
                                        "Command timed out after {timeout_secs}s; process group terminated.\n\nOutput so far:\n{all_output}"
                                    ));
                                }
                            }
                        }
                    }

                    send_shell_run_omitted_notice(&mut live_log_omitted);

                    let exit_code = exit_status
                        .or_else(|| child.wait().ok())
                        .map(|s| s.code().unwrap_or(-1))
                        .unwrap_or(-1);
                    if exit_code != 0 {
                        all_output.push_str(&format!("\n[exit {exit_code}]"));
                    }
                    finish_and_close_shell_command_event(
                        &command_event,
                        shell_status_from_exit_code(exit_code),
                        Some(exit_code),
                    );
                    if all_output.is_empty() {
                        all_output = "(no output)".to_string();
                    }
                    Ok(all_output)
                }

                "start" => {
                    let command = args
                        .get("command")
                        .and_then(|v| v.as_str())
                        .ok_or("command required")?
                        .to_string();
                    let cwd = args
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .unwrap_or(project_path)
                        .to_string();
                    validate_command(&command)?;
                    if let Some(api) = live::PtyApi::for_host(base_url, token)? {
                        let started = api.start(&command, &cwd)?;
                        let command_id = started["command_session_id"]
                            .as_str()
                            .ok_or("Missing command session id")?
                            .to_string();
                        std::thread::spawn(move || {
                            let _ = api.forward(&command_id, None);
                        });
                        return Ok(started.to_string());
                    }
                    {
                        let sessions = shell_sessions().lock().unwrap();
                        if sessions.len() >= 10 {
                            return Err(
                                "Too many active shell sessions (max 10). Kill some first."
                                    .to_string(),
                            );
                        }
                    }
                    let session_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
                    let command_event =
                        start_shell_command_event(Some(session_id.clone()), &command, &cwd);
                    let auth_env = github_cli_auth_env(base_url, token);
                    let mut cmd = flamel::mcp_parent_guard::command("sh");
                    cmd.arg("-c")
                        .arg(&command)
                        .current_dir(&cwd)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped());
                    inject_shell_auth_env(&mut cmd, &auth_env);
                    configure_shell_child(&mut cmd);
                    let mut child = match cmd.spawn() {
                        Ok(child) => child,
                        Err(e) => {
                            let error = format!("Failed to spawn: {e}");
                            emit_shell_error_event(&command_event, &error);
                            finish_and_close_shell_command_event(&command_event, "failed", None);
                            return Err(error);
                        }
                    };
                    let output_buf = Arc::new(Mutex::new(String::new()));
                    if let Some(stdout) = child.stdout.take() {
                        let buf = Arc::clone(&output_buf);
                        let command_event = command_event.clone();
                        std::thread::spawn(move || {
                            let mut reader = io::BufReader::new(stdout);
                            let mut line = String::new();
                            loop {
                                line.clear();
                                match reader.read_line(&mut line) {
                                    Ok(0) | Err(_) => break,
                                    Ok(_) => {
                                        let event_line =
                                            line.trim_end_matches(|c| c == '\r' || c == '\n');
                                        emit_shell_output_event(
                                            &command_event,
                                            "stdout",
                                            event_line,
                                        );
                                        append_capped_shell_output(&mut buf.lock().unwrap(), &line);
                                    }
                                }
                            }
                        });
                    }
                    if let Some(stderr) = child.stderr.take() {
                        let buf = Arc::clone(&output_buf);
                        let command_event = command_event.clone();
                        std::thread::spawn(move || {
                            let mut reader = io::BufReader::new(stderr);
                            let mut line = String::new();
                            loop {
                                line.clear();
                                match reader.read_line(&mut line) {
                                    Ok(0) | Err(_) => break,
                                    Ok(_) => {
                                        let event_line =
                                            line.trim_end_matches(|c| c == '\r' || c == '\n');
                                        emit_shell_output_event(
                                            &command_event,
                                            "stderr",
                                            event_line,
                                        );
                                        let mut b = buf.lock().unwrap();
                                        append_capped_shell_output(
                                            &mut b,
                                            &format!("[stderr] {line}"),
                                        );
                                    }
                                }
                            }
                        });
                    }
                    let stdin = child.stdin.take();
                    let now = std::time::Instant::now();
                    shell_sessions().lock().unwrap().insert(
                        session_id.clone(),
                        ShellSession {
                            child,
                            stdin,
                            output_buf,
                            started_at: now,
                            last_read: now,
                            cwd: cwd.clone(),
                            command: command.clone(),
                            command_event,
                            event_closed: false,
                        },
                    );
                    Ok(
                        json!({"session_id": session_id, "command": command, "cwd": cwd})
                            .to_string(),
                    )
                }

                "read" => {
                    let sid = args
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .ok_or("session_id required")?;
                    let timeout_ms = args
                        .get("timeout_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(100);
                    if let Some(id) = live::terminal_id(sid)? {
                        let api = live::PtyApi::for_host(base_url, token)?
                            .ok_or("Terminal belongs to another execution host")?;
                        std::thread::sleep(std::time::Duration::from_millis(timeout_ms.min(2000)));
                        return api
                            .call(
                                reqwest::Method::GET,
                                &format!("/api/terminal/{id}/output"),
                                None,
                            )
                            .map(|value| value.to_string());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(timeout_ms));
                    let mut sessions = shell_sessions().lock().unwrap();
                    let session = sessions.get_mut(sid).ok_or("Session not found")?;
                    session.last_read = std::time::Instant::now();
                    let mut buf = session.output_buf.lock().unwrap();
                    let output = if buf.is_empty() {
                        "(no new output)".to_string()
                    } else {
                        let s = buf.clone();
                        buf.clear();
                        s
                    };
                    drop(buf);
                    let status = match session.child.try_wait() {
                        Ok(Some(s)) => {
                            let exit_code = s.code().unwrap_or(-1);
                            if !session.event_closed {
                                finish_and_close_shell_command_event(
                                    &session.command_event,
                                    shell_status_from_exit_code(exit_code),
                                    Some(exit_code),
                                );
                                session.event_closed = true;
                            }
                            Some(exit_code)
                        }
                        _ => None,
                    };
                    Ok(
                        json!({"output": output, "exited": status.is_some(), "exit_code": status})
                            .to_string(),
                    )
                }

                "store_secret" | "delete_secret" | "secret_status" => {
                    let name = args
                        .get("secret_name")
                        .and_then(Value::as_str)
                        .ok_or("secret_name required")?;
                    let api_action = match action {
                        "store_secret" => "store",
                        "delete_secret" => "delete",
                        _ => "status",
                    };
                    let mut body = json!({"action": api_action, "name": name});
                    if action == "store_secret" {
                        let value = args
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or("text required")?;
                        body["value"] = json!(value);
                    }
                    let response = call_local_secret_api(&body, base_url, token)?;
                    Ok(match action {
                        "secret_status" => json!({
                            "ok": true,
                            "name": name,
                            "stored": response.get("stored").and_then(Value::as_bool).unwrap_or(false),
                        }),
                        "delete_secret" => json!({
                            "ok": true,
                            "name": name,
                            "deleted": response.get("deleted").and_then(Value::as_bool).unwrap_or(false),
                        }),
                        _ => json!({"ok": true, "name": name, "stored": true}),
                    }
                    .to_string())
                }

                "write" => {
                    let sid = args
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .ok_or("session_id required")?;
                    let secret_ref = args.get("secret_ref").and_then(Value::as_str);
                    if secret_ref.is_some() && args.get("text").is_some() {
                        return Err("text and secret_ref are mutually exclusive".into());
                    }
                    let mut text = if let Some(name) = secret_ref {
                        call_local_secret_api(
                            &json!({"action": "load", "name": name}),
                            base_url,
                            token,
                        )?
                        .get("value")
                        .and_then(Value::as_str)
                        .ok_or("Local secret response did not contain a value")?
                        .to_string()
                    } else {
                        args.get("text")
                            .and_then(Value::as_str)
                            .ok_or("text or secret_ref required")?
                            .to_string()
                    };
                    if args
                        .get("append_newline")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        text.push('\n');
                    }
                    let sensitive = secret_ref.is_some()
                        || args
                            .get("sensitive")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                    if let Some(id) = live::terminal_id(sid)? {
                        let api = live::PtyApi::for_host(base_url, token)?
                            .ok_or("Terminal belongs to another execution host")?;
                        api.call(
                            reqwest::Method::POST,
                            &format!("/api/terminal/{id}/input"),
                            Some(&json!({"text": text})),
                        )?;
                        return Ok(json!({"ok": true, "sensitive": sensitive}).to_string());
                    }
                    let mut sessions = shell_sessions().lock().unwrap();
                    let session = sessions.get_mut(sid).ok_or("Session not found")?;
                    let stdin = session.stdin.as_mut().ok_or("stdin not available")?;
                    stdin
                        .write_all(text.as_bytes())
                        .map_err(|e| format!("write failed: {e}"))?;
                    stdin.flush().map_err(|e| format!("flush failed: {e}"))?;
                    // Raw stdin can contain passwords even without a caller's
                    // sensitive flag. Never copy it into the console journal.
                    Ok(if sensitive {
                        json!({"ok": true, "sensitive": true}).to_string()
                    } else {
                        json!({"ok": true, "bytes_written": text.len()}).to_string()
                    })
                }

                "kill" => {
                    let sid = args
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .ok_or("session_id required")?;
                    if let Some(id) = live::terminal_id(sid)? {
                        let api = live::PtyApi::for_host(base_url, token)?
                            .ok_or("Terminal belongs to another execution host")?;
                        return api
                            .call(
                                reqwest::Method::DELETE,
                                &format!("/api/terminal/{id}"),
                                None,
                            )
                            .map(|value| value.to_string());
                    }
                    let mut sessions = shell_sessions().lock().unwrap();
                    let mut session = sessions.remove(sid).ok_or("Session not found")?;
                    terminate_shell_child(&mut session.child);
                    if !session.event_closed {
                        finish_and_close_shell_command_event(
                            &session.command_event,
                            "cancelled",
                            None,
                        );
                    }
                    Ok(json!({"ok": true, "session_id": sid}).to_string())
                }

                "status" => {
                    let sid = args
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .ok_or("session_id required")?;
                    if let Some(id) = live::terminal_id(sid)? {
                        let api = live::PtyApi::for_host(base_url, token)?
                            .ok_or("Terminal belongs to another execution host")?;
                        let status = api.call(
                            reqwest::Method::GET,
                            &format!("/api/terminal/{id}/status"),
                            None,
                        )?;
                        return Ok(json!({"session_id": sid, "command": status["command"], "cwd": status["cwd"], "status": status}).to_string());
                    }
                    let mut sessions = shell_sessions().lock().unwrap();
                    let session = sessions.get_mut(sid).ok_or("Session not found")?;
                    let status = match session.child.try_wait() {
                        Ok(Some(s)) => {
                            let exit_code = s.code().unwrap_or(-1);
                            if !session.event_closed {
                                finish_and_close_shell_command_event(
                                    &session.command_event,
                                    shell_status_from_exit_code(exit_code),
                                    Some(exit_code),
                                );
                                session.event_closed = true;
                            }
                            json!({"running": false, "exit_code": exit_code})
                        }
                        Ok(None) => {
                            json!({"running": true, "uptime_secs": session.started_at.elapsed().as_secs()})
                        }
                        Err(e) => {
                            let error = e.to_string();
                            if !session.event_closed {
                                emit_shell_error_event(&session.command_event, &error);
                                finish_and_close_shell_command_event(
                                    &session.command_event,
                                    "failed",
                                    None,
                                );
                                session.event_closed = true;
                            }
                            json!({"running": false, "error": error})
                        }
                    };
                    Ok(json!({"session_id": sid, "command": session.command, "cwd": session.cwd, "status": status}).to_string())
                }

                "multi" => {
                    let commands = coerce_array(args.get("commands"));
                    let cmds_arr = commands
                        .as_array()
                        .ok_or("commands required — array of strings or {command, cwd?} objects")?;
                    if cmds_arr.len() > 5 {
                        return Err("max 5 concurrent commands".to_string());
                    }
                    let timeout_secs = args
                        .get("timeout_secs")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(120);
                    let mut jobs = Vec::new();
                    for cmd_val in cmds_arr {
                        let (cmd, cwd) = match cmd_val {
                            Value::String(s) => (s.clone(), project_path.to_string()),
                            Value::Object(obj) => {
                                let c = obj
                                    .get("command")
                                    .and_then(|v| v.as_str())
                                    .ok_or("command field required")?
                                    .to_string();
                                let d = obj
                                    .get("cwd")
                                    .and_then(|v| v.as_str())
                                    .map(|c| resolve_path(c, project_path))
                                    .unwrap_or_else(|| project_path.to_string());
                                (c, d)
                            }
                            _ => {
                                return Err(
                                    "each command must be a string or {command, cwd}".to_string()
                                );
                            }
                        };
                        jobs.push((cmd, cwd));
                    }
                    for (command, _) in &jobs {
                        validate_command(command)?;
                    }
                    // Use the same streaming, bounded-output and process-group
                    // timeout implementation as single commands. Never detach
                    // an output() thread whose child can outlive a timeout.
                    let results = std::thread::scope(|scope| {
                        let handles: Vec<_> = jobs.into_iter().map(|(command, cwd)| scope.spawn(move || {
                            let args = json!({"action": "run", "command": command, "cwd": cwd, "timeout_secs": timeout_secs});
                            let result = handle("shell", &args, base_url, token, _session_id, project_path);
                            match result {
                                Ok(output) => format!("$ {command}\n{}", cap_chars(&output, 2000, "...[truncated]")),
                                Err(error) => format!("$ {command}\n[error] {error}"),
                            }
                        })).collect();
                        handles
                            .into_iter()
                            .map(|handle| {
                                handle
                                    .join()
                                    .unwrap_or_else(|_| "[error] command worker failed".to_string())
                            })
                            .collect::<Vec<_>>()
                    });
                    Ok(results.join("\n\n---\n\n"))
                }

                _ => Err(format!(
                    "Unknown shell action '{action}'. Valid: run, start, read, write, kill, status, multi"
                )),
            }
        }
        _ => Err(format!("Unknown shell tool: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_multi_uses_bounded_timeout_and_unicode_safe_results() {
        let started = std::time::Instant::now();
        let output = handle("shell", &json!({"action":"multi", "commands":["sleep 30", "printf '你好\\n'"], "timeout_secs":1}),
            "https://coordinator.example.invalid", "", "test", std::env::temp_dir().to_str().unwrap()).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert!(output.contains("timed out after 1s; process group terminated"));
        assert!(output.contains("你好"));
    }

    #[test]
    fn output_tail_cap_respects_unicode_boundaries() {
        let mut output = String::new();
        append_capped_shell_output(&mut output, &"界".repeat(100_000));
        assert!(output.len() <= SHELL_RUN_OUTPUT_CAP);
        assert!(output.contains("truncated to last 256KB"));
        assert!(output.ends_with("界\n"));
    }
}

fn validate_command(command: &str) -> Result<(), String> {
    if !unrestricted_shell_enabled() {
        let trimmed = command.trim_start();
        let first_word = trimmed.split_whitespace().next().unwrap_or("");
        if matches!(first_word, "git") {
            return Err("Do not run Git through shell; use the typed Flamel git tool.".to_string());
        }
        if matches!(first_word, "cat" | "head" | "tail") {
            return Err(format!("Do NOT use {first_word} — use file_read instead."));
        }
        if matches!(first_word, "grep" | "rg") {
            return Err(
                "Do NOT use grep/rg — use code_nav(action=\"search\") or flamel_search instead."
                    .to_string(),
            );
        }
        if matches!(first_word, "find" | "fd") && trimmed.contains("-name") {
            return Err("Do NOT use find -name — use flamel_glob instead.".to_string());
        }
        if matches!(first_word, "sed") && (trimmed.contains("-i") || trimmed.contains("--in-place"))
        {
            return Err("Do NOT use sed -i — use flamel_replace instead.".to_string());
        }
        if matches!(first_word, "python3" | "python") {
            let is_file_write = trimmed.contains("open(")
                && (trimmed.contains("'w'")
                    || trimmed.contains("\"w\"")
                    || trimmed.contains("'w+'")
                    || trimmed.contains("\"w+\""))
                || trimmed.contains(".write(")
                || (trimmed.contains(">>") && trimmed.contains("open("));
            if is_file_write {
                return Err(
                    "Do NOT use Python to write files — use file_edit tools instead.".to_string(),
                );
            }
        }
        if matches!(first_word, "tee") || (trimmed.contains('>') && !trimmed.starts_with('#')) {
            let has_redirect = trimmed.split_whitespace().any(|tok| {
                tok == ">" || (tok.starts_with('>') && tok.len() > 1 && !tok.starts_with(">>"))
            });
            if has_redirect {
                return Err(
                    "Do NOT use shell redirection (>) — use create_text_file instead.".to_string(),
                );
            }
        }
    }
    Ok(())
}
