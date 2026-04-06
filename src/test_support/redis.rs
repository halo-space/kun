use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, oneshot};

#[derive(Default)]
struct TestRedisState {
    strings: BTreeMap<String, String>,
    hashes: BTreeMap<String, BTreeMap<String, String>>,
    sets: BTreeMap<String, BTreeSet<String>>,
    sorted_sets: BTreeMap<String, BTreeMap<String, i64>>,
}

#[derive(Default)]
struct TestRedisServerState {
    redis: TestRedisState,
    commands: Vec<Vec<String>>,
}

pub(crate) async fn spawn_redis_server() -> (
    String,
    oneshot::Receiver<Vec<Vec<String>>>,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (commands_tx, commands_rx) = oneshot::channel();

    let server_handle = tokio::spawn(async move {
        let state = Arc::new(Mutex::new(TestRedisServerState::default()));
        let active_connections = Arc::new(AtomicUsize::new(0));
        let mut connection_tasks = tokio::task::JoinSet::new();

        loop {
            let accept_result =
                tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                    .await;

            let Ok(accept_result) = accept_result else {
                if active_connections.load(Ordering::SeqCst) == 0 {
                    break;
                }
                continue;
            };
            let (mut stream, _) = accept_result?;
            let state = Arc::clone(&state);
            let active_connections = Arc::clone(&active_connections);
            active_connections.fetch_add(1, Ordering::SeqCst);

            connection_tasks.spawn(async move {
                let result = handle_client(&mut stream, state).await;
                active_connections.fetch_sub(1, Ordering::SeqCst);
                result
            });
        }

        while let Some(result) = connection_tasks.join_next().await {
            result??;
        }

        let commands = {
            let mut state = state.lock().await;
            std::mem::take(&mut state.commands)
        };
        commands_tx.send(commands).ok();
        Ok(())
    });

    (address.to_string(), commands_rx, server_handle)
}

async fn handle_client(
    stream: &mut TcpStream,
    state: Arc<Mutex<TestRedisServerState>>,
) -> Result<(), std::io::Error> {
    while let Some(command) = read_resp_command(stream).await? {
        let reply = {
            let mut state = state.lock().await;
            let reply = handle_command(&mut state.redis, &command)?;
            if !is_implicit_client_command(&command) {
                state.commands.push(command);
            }
            reply
        };
        stream.write_all(&reply).await?;
    }
    Ok(())
}

fn handle_command(
    state: &mut TestRedisState,
    command: &[String],
) -> Result<Vec<u8>, std::io::Error> {
    let Some(name) = command.first().map(String::as_str) else {
        return Ok(error_reply("ERR empty command"));
    };

    match name {
        "AUTH" | "SELECT" => Ok(simple_reply("OK")),
        "CLIENT" => Ok(simple_reply("OK")),
        "EVAL" => handle_eval_command(state, command),
        "SET" => {
            if command.len() != 3 {
                return Ok(error_reply("ERR wrong number of arguments for SET"));
            }
            state.strings.insert(command[1].clone(), command[2].clone());
            Ok(simple_reply("OK"))
        }
        "GET" => {
            if command.len() != 2 {
                return Ok(error_reply("ERR wrong number of arguments for GET"));
            }
            Ok(bulk_reply(state.strings.get(&command[1]).cloned()))
        }
        "DEL" => {
            if command.len() < 2 {
                return Ok(error_reply("ERR wrong number of arguments for DEL"));
            }
            let mut deleted = 0_i64;
            for key in &command[1..] {
                deleted += i64::from(state.strings.remove(key).is_some());
                deleted += i64::from(state.hashes.remove(key).is_some());
                deleted += i64::from(state.sets.remove(key).is_some());
                deleted += i64::from(state.sorted_sets.remove(key).is_some());
            }
            Ok(integer_reply(deleted))
        }
        "INCR" => {
            if command.len() != 2 {
                return Ok(error_reply("ERR wrong number of arguments for INCR"));
            }
            let value = state
                .strings
                .get(&command[1])
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0)
                + 1;
            state.strings.insert(command[1].clone(), value.to_string());
            Ok(integer_reply(value))
        }
        "HSET" => {
            if command.len() < 4 || command.len() % 2 != 0 {
                return Ok(error_reply("ERR wrong number of arguments for HSET"));
            }
            let fields = state.hashes.entry(command[1].clone()).or_default();
            let mut created = 0_i64;
            for pair in command[2..].chunks(2) {
                if fields.insert(pair[0].clone(), pair[1].clone()).is_none() {
                    created += 1;
                }
            }
            Ok(integer_reply(created))
        }
        "HGET" => {
            if command.len() != 3 {
                return Ok(error_reply("ERR wrong number of arguments for HGET"));
            }
            let value = state
                .hashes
                .get(&command[1])
                .and_then(|fields| fields.get(&command[2]).cloned());
            Ok(bulk_reply(value))
        }
        "HMGET" => {
            if command.len() < 3 {
                return Ok(error_reply("ERR wrong number of arguments for HMGET"));
            }
            let values = command[2..]
                .iter()
                .map(|field| {
                    state
                        .hashes
                        .get(&command[1])
                        .and_then(|fields| fields.get(field).cloned())
                })
                .collect::<Vec<_>>();
            Ok(array_reply_optional(values))
        }
        "HGETALL" => {
            if command.len() != 2 {
                return Ok(error_reply("ERR wrong number of arguments for HGETALL"));
            }
            let mut values = Vec::new();
            if let Some(fields) = state.hashes.get(&command[1]) {
                for (field, value) in fields {
                    values.push(field.clone());
                    values.push(value.clone());
                }
            }
            Ok(array_reply(values))
        }
        "HDEL" => {
            if command.len() < 3 {
                return Ok(error_reply("ERR wrong number of arguments for HDEL"));
            }
            let Some(fields) = state.hashes.get_mut(&command[1]) else {
                return Ok(integer_reply(0));
            };
            let mut deleted = 0_i64;
            for field in &command[2..] {
                if fields.remove(field).is_some() {
                    deleted += 1;
                }
            }
            Ok(integer_reply(deleted))
        }
        "SADD" => {
            if command.len() < 3 {
                return Ok(error_reply("ERR wrong number of arguments for SADD"));
            }
            let members = state.sets.entry(command[1].clone()).or_default();
            let mut added = 0_i64;
            for member in &command[2..] {
                if members.insert(member.clone()) {
                    added += 1;
                }
            }
            Ok(integer_reply(added))
        }
        "SMEMBERS" => {
            if command.len() != 2 {
                return Ok(error_reply("ERR wrong number of arguments for SMEMBERS"));
            }
            let members = state
                .sets
                .get(&command[1])
                .map(|members| members.iter().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            Ok(array_reply(members))
        }
        "SREM" => {
            if command.len() < 3 {
                return Ok(error_reply("ERR wrong number of arguments for SREM"));
            }
            let Some(members) = state.sets.get_mut(&command[1]) else {
                return Ok(integer_reply(0));
            };
            let mut removed = 0_i64;
            for member in &command[2..] {
                if members.remove(member) {
                    removed += 1;
                }
            }
            Ok(integer_reply(removed))
        }
        "SCARD" => {
            if command.len() != 2 {
                return Ok(error_reply("ERR wrong number of arguments for SCARD"));
            }
            let count = state.sets.get(&command[1]).map_or(0, BTreeSet::len);
            Ok(integer_reply(i64::try_from(count).unwrap_or_default()))
        }
        "ZADD" => {
            if command.len() < 4 || command.len() % 2 != 0 {
                return Ok(error_reply("ERR wrong number of arguments for ZADD"));
            }
            let members = state.sorted_sets.entry(command[1].clone()).or_default();
            let mut added = 0_i64;
            for pair in command[2..].chunks(2) {
                let score = pair[0].parse::<i64>().map_err(|error| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("invalid ZADD score: {error}"),
                    )
                })?;
                if members.insert(pair[1].clone(), score).is_none() {
                    added += 1;
                }
            }
            Ok(integer_reply(added))
        }
        "ZRANGE" => {
            if command.len() != 4 && command.len() != 5 {
                return Ok(error_reply("ERR wrong number of arguments for ZRANGE"));
            }
            let members = sorted_members(state.sorted_sets.get(&command[1]));
            let withscores = command
                .get(4)
                .is_some_and(|option| option.eq_ignore_ascii_case("WITHSCORES"));
            if command.len() == 5 && !withscores {
                return Ok(error_reply("ERR syntax error"));
            }
            let range = slice_range_entries(&members, &command[2], &command[3])?;
            if withscores {
                let mut values = Vec::with_capacity(range.len() * 2);
                for (member, score) in range {
                    values.push(member);
                    values.push(score.to_string());
                }
                Ok(array_reply(values))
            } else {
                Ok(array_reply(
                    range.into_iter().map(|(member, _)| member).collect(),
                ))
            }
        }
        "ZRANGEBYSCORE" => {
            if command.len() != 4 {
                return Ok(error_reply(
                    "ERR wrong number of arguments for ZRANGEBYSCORE",
                ));
            }
            let members = sorted_members(state.sorted_sets.get(&command[1]));
            let min = parse_score_bound(&command[2])?;
            let max = parse_score_bound(&command[3])?;
            let selected = members
                .into_iter()
                .filter(|(_, score)| *score >= min && *score <= max)
                .map(|(member, _)| member)
                .collect::<Vec<_>>();
            Ok(array_reply(selected))
        }
        "ZREM" => {
            if command.len() < 3 {
                return Ok(error_reply("ERR wrong number of arguments for ZREM"));
            }
            let Some(members) = state.sorted_sets.get_mut(&command[1]) else {
                return Ok(integer_reply(0));
            };
            let mut removed = 0_i64;
            for member in &command[2..] {
                if members.remove(member).is_some() {
                    removed += 1;
                }
            }
            Ok(integer_reply(removed))
        }
        "ZCARD" => {
            if command.len() != 2 {
                return Ok(error_reply("ERR wrong number of arguments for ZCARD"));
            }
            let count = state.sorted_sets.get(&command[1]).map_or(0, BTreeMap::len);
            Ok(integer_reply(i64::try_from(count).unwrap_or_default()))
        }
        _ => Ok(error_reply("ERR unsupported test command")),
    }
}

fn handle_eval_command(
    state: &mut TestRedisState,
    command: &[String],
) -> Result<Vec<u8>, std::io::Error> {
    if command.len() < 3 {
        return Ok(error_reply("ERR wrong number of arguments for EVAL"));
    }

    let num_keys = command[2].parse::<usize>().map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid EVAL key count: {error}"),
        )
    })?;
    if command.len() < 3 + num_keys {
        return Ok(error_reply("ERR wrong number of arguments for EVAL"));
    }

    let script = command[1].as_str();
    let keys = &command[3..3 + num_keys];
    let args = &command[3 + num_keys..];

    if script.contains("kun:scheduler:enqueue_v1") {
        return eval_scheduler_enqueue(state, keys, args);
    }
    if script.contains("kun:scheduler:reclaim_v1") || script.contains("kun:scheduler:reclaim_v2") {
        return eval_scheduler_reclaim(state, keys, args);
    }
    if script.contains("kun:scheduler:claim_ready_v1")
        || script.contains("kun:scheduler:claim_ready_v2")
    {
        return eval_scheduler_claim_ready(state, keys, args);
    }
    if script.contains("kun:scheduler:complete_v1")
        || script.contains("kun:scheduler:complete_v2")
        || script.contains("kun:scheduler:complete_v3")
        || script.contains("kun:scheduler:complete_v4")
    {
        return eval_scheduler_complete(state, keys, args);
    }
    if script.contains("kun:scheduler:requeue_v1")
        || script.contains("kun:scheduler:requeue_v2")
        || script.contains("kun:scheduler:requeue_v3")
        || script.contains("kun:scheduler:requeue_v4")
    {
        return eval_scheduler_requeue(state, keys, args);
    }
    if script.contains("kun:scheduler:heartbeat_v1")
        || script.contains("kun:scheduler:heartbeat_v2")
        || script.contains("kun:scheduler:heartbeat_v3")
    {
        return eval_scheduler_heartbeat(state, keys, args);
    }
    if script.contains("kun:scheduler:release_inflight_v1") {
        return eval_scheduler_release_inflight(state, keys, args);
    }

    Ok(error_reply("ERR unsupported test script"))
}

fn eval_scheduler_enqueue(
    state: &mut TestRedisState,
    keys: &[String],
    args: &[String],
) -> Result<Vec<u8>, std::io::Error> {
    if keys.len() != 5 || args.len() != 3 {
        return Ok(error_reply("ERR invalid enqueue script args"));
    }

    let task_id = args[0].as_str();
    let task_json = args[1].clone();
    let ready_at = args[2].as_str();

    hash_set(state, &keys[0], task_id, task_json);
    if ready_at.is_empty() {
        push_ready(state, &keys[1], &keys[2], &keys[4], task_id)?;
    } else {
        sorted_set_insert(
            state,
            &keys[3],
            task_id,
            ready_at.parse::<i64>().map_err(int_error)?,
        );
    }

    Ok(integer_reply(1))
}

fn eval_scheduler_reclaim(
    state: &mut TestRedisState,
    keys: &[String],
    args: &[String],
) -> Result<Vec<u8>, std::io::Error> {
    if keys.len() != 10 || args.len() != 1 {
        return Ok(error_reply("ERR invalid reclaim script args"));
    }

    let now = args[0].parse::<i64>().map_err(int_error)?;
    let reclaimed = reclaim_expired_inflight(state, keys, now)?;
    Ok(integer_reply(i64::try_from(reclaimed).unwrap_or_default()))
}

fn eval_scheduler_claim_ready(
    state: &mut TestRedisState,
    keys: &[String],
    args: &[String],
) -> Result<Vec<u8>, std::io::Error> {
    if keys.len() != 10 || args.len() != 4 {
        return Ok(error_reply("ERR invalid claim script args"));
    }

    let now = args[0].parse::<i64>().map_err(int_error)?;
    reclaim_expired_inflight(state, keys, now)?;
    promote_delayed(state, keys, now)?;

    let Some((task_id, task_json)) = choose_best_ready_task(state, &keys[0], &keys[1], &keys[2])?
    else {
        return Ok(bulk_reply(None));
    };

    set_remove(state, &keys[1], task_id.as_str());
    hash_delete(state, &keys[2], task_id.as_str());
    set_insert(state, &keys[4], task_id.as_str());
    sorted_set_remove(state, &keys[5], task_id.as_str());
    hash_set(state, &keys[6], task_id.as_str(), args[2].clone());
    hash_set(state, &keys[7], task_id.as_str(), args[3].clone());

    if !args[1].is_empty() {
        let deadline = now.saturating_add(args[1].parse::<i64>().map_err(int_error)?);
        sorted_set_insert(state, &keys[5], task_id.as_str(), deadline);
    }

    Ok(array_reply_optional(vec![
        Some(task_json),
        Some(args[3].clone()),
    ]))
}

fn eval_scheduler_complete(
    state: &mut TestRedisState,
    keys: &[String],
    args: &[String],
) -> Result<Vec<u8>, std::io::Error> {
    if keys.len() != 12 || args.len() != 6 {
        return Ok(error_reply("ERR invalid complete script args"));
    }

    let task_id = args[0].as_str();
    if !lease_matches(state, &keys[6], &keys[7], task_id, &args[1], &args[2]) {
        return Ok(integer_reply(lease_result_code(
            state, &keys[6], &keys[7], task_id, &args[1], &args[2],
        )));
    }
    sync_worker_runtime_meta(
        state, &keys[8], &keys[9], &keys[10], &keys[11], &args[1], &args[3], &args[4], &args[5],
    );
    sorted_set_remove(state, &keys[5], task_id);
    set_remove(state, &keys[4], task_id);
    set_remove(state, &keys[1], task_id);
    sorted_set_remove(state, &keys[3], task_id);
    hash_delete(state, &keys[2], task_id);
    hash_delete(state, &keys[6], task_id);
    hash_delete(state, &keys[7], task_id);
    hash_delete(state, &keys[0], task_id);

    Ok(integer_reply(1))
}

fn eval_scheduler_requeue(
    state: &mut TestRedisState,
    keys: &[String],
    args: &[String],
) -> Result<Vec<u8>, std::io::Error> {
    if keys.len() != 13 || args.len() != 7 {
        return Ok(error_reply("ERR invalid requeue script args"));
    }

    let task_id = args[0].as_str();
    let now = args[1].parse::<i64>().map_err(int_error)?;
    if !lease_matches(state, &keys[6], &keys[7], task_id, &args[2], &args[3]) {
        return Ok(integer_reply(lease_result_code(
            state, &keys[6], &keys[7], task_id, &args[2], &args[3],
        )));
    }
    sync_worker_runtime_meta(
        state, &keys[9], &keys[10], &keys[11], &keys[12], &args[2], &args[4], &args[5], &args[6],
    );
    let task_json = state
        .hashes
        .get(&keys[0])
        .and_then(|tasks| tasks.get(task_id))
        .cloned();

    set_remove(state, &keys[4], task_id);
    sorted_set_remove(state, &keys[5], task_id);
    hash_delete(state, &keys[6], task_id);
    hash_delete(state, &keys[7], task_id);
    set_remove(state, &keys[1], task_id);
    hash_delete(state, &keys[2], task_id);
    sorted_set_remove(state, &keys[3], task_id);

    let Some(task_json) = task_json else {
        return Ok(integer_reply(0));
    };

    route_task(
        state, &keys[1], &keys[2], &keys[3], &keys[8], task_id, &task_json, now,
    )?;
    Ok(integer_reply(1))
}

fn eval_scheduler_heartbeat(
    state: &mut TestRedisState,
    keys: &[String],
    args: &[String],
) -> Result<Vec<u8>, std::io::Error> {
    if keys.len() != 8 || args.len() != 7 {
        return Ok(error_reply("ERR invalid heartbeat script args"));
    }

    let task_id = args[0].as_str();
    let deadline = args[1].parse::<i64>().map_err(int_error)?;
    if !lease_matches(state, &keys[2], &keys[3], task_id, &args[2], &args[3]) {
        return Ok(integer_reply(lease_result_code(
            state, &keys[2], &keys[3], task_id, &args[2], &args[3],
        )));
    }
    if !set_remove(state, &keys[0], task_id) {
        return Ok(integer_reply(0));
    }
    set_insert(state, &keys[0], task_id);
    sorted_set_insert(state, &keys[1], task_id, deadline);
    sync_worker_runtime_meta(
        state, &keys[4], &keys[5], &keys[6], &keys[7], &args[2], &args[4], &args[5], &args[6],
    );
    Ok(integer_reply(1))
}

fn sync_worker_runtime_meta(
    state: &mut TestRedisState,
    workers_key: &str,
    worker_seen_key: &str,
    worker_lease_timeout_key: &str,
    worker_heartbeat_interval_key: &str,
    worker_id: &str,
    worker_seen: &str,
    lease_timeout: &str,
    heartbeat_interval: &str,
) {
    set_insert(state, workers_key, worker_id);
    hash_set(state, worker_seen_key, worker_id, worker_seen.to_string());
    if lease_timeout.is_empty() {
        hash_delete(state, worker_lease_timeout_key, worker_id);
    } else {
        hash_set(
            state,
            worker_lease_timeout_key,
            worker_id,
            lease_timeout.to_string(),
        );
    }
    if heartbeat_interval.is_empty() {
        hash_delete(state, worker_heartbeat_interval_key, worker_id);
    } else {
        hash_set(
            state,
            worker_heartbeat_interval_key,
            worker_id,
            heartbeat_interval.to_string(),
        );
    }
}

fn eval_scheduler_release_inflight(
    state: &mut TestRedisState,
    keys: &[String],
    args: &[String],
) -> Result<Vec<u8>, std::io::Error> {
    if keys.len() != 9 || args.len() != 2 {
        return Ok(error_reply("ERR invalid release inflight script args"));
    }

    let now = args[0].parse::<i64>().map_err(int_error)?;
    let worker_id = args[1].as_str();
    let task_ids = state
        .hashes
        .get(&keys[6])
        .map(|workers| {
            workers
                .iter()
                .filter_map(|(task_id, owner)| {
                    if owner == worker_id {
                        Some(task_id.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut released = 0usize;
    for task_id in task_ids {
        let task_json = state
            .hashes
            .get(&keys[0])
            .and_then(|tasks| tasks.get(task_id.as_str()))
            .cloned();
        let removed = set_remove(state, &keys[4], task_id.as_str());
        sorted_set_remove(state, &keys[5], task_id.as_str());
        hash_delete(state, &keys[6], task_id.as_str());
        hash_delete(state, &keys[7], task_id.as_str());

        let Some(task_json) = task_json else {
            continue;
        };
        if !removed {
            continue;
        }

        route_task(
            state,
            &keys[1],
            &keys[2],
            &keys[3],
            &keys[8],
            task_id.as_str(),
            &task_json,
            now,
        )?;
        released += 1;
    }

    Ok(integer_reply(i64::try_from(released).unwrap_or_default()))
}

fn reclaim_expired_inflight(
    state: &mut TestRedisState,
    keys: &[String],
    now: i64,
) -> Result<usize, std::io::Error> {
    let expired_ids = sorted_members(state.sorted_sets.get(&keys[5]))
        .into_iter()
        .filter(|(_, score)| *score <= now)
        .map(|(task_id, _)| task_id)
        .collect::<Vec<_>>();

    let mut reclaimed = 0usize;
    for task_id in expired_ids {
        let removed_deadline = sorted_set_remove(state, &keys[5], task_id.as_str());
        let removed_inflight = set_remove(state, &keys[4], task_id.as_str());
        hash_delete(state, &keys[6], task_id.as_str());
        hash_delete(state, &keys[7], task_id.as_str());
        if !(removed_deadline || removed_inflight) {
            continue;
        }

        let task_json = state
            .hashes
            .get(&keys[0])
            .and_then(|tasks| tasks.get(task_id.as_str()))
            .cloned();
        let Some(task_json) = task_json else {
            continue;
        };

        route_task(
            state,
            &keys[1],
            &keys[2],
            &keys[3],
            &keys[8],
            task_id.as_str(),
            &task_json,
            now,
        )?;
        reclaimed += 1;
    }

    for _ in 0..reclaimed {
        increment_string_counter(state, &keys[9])?;
    }

    Ok(reclaimed)
}

fn promote_delayed(
    state: &mut TestRedisState,
    keys: &[String],
    now: i64,
) -> Result<(), std::io::Error> {
    let delayed_ids = sorted_members(state.sorted_sets.get(&keys[3]))
        .into_iter()
        .filter(|(_, score)| *score <= now)
        .map(|(task_id, _)| task_id)
        .collect::<Vec<_>>();

    for task_id in delayed_ids {
        if !sorted_set_remove(state, &keys[3], task_id.as_str()) {
            continue;
        }
        if task_exists(state, &keys[0], task_id.as_str()) {
            push_ready(state, &keys[1], &keys[2], &keys[8], task_id.as_str())?;
        }
    }

    Ok(())
}

fn choose_best_ready_task(
    state: &mut TestRedisState,
    tasks_key: &str,
    ready_key: &str,
    ready_order_key: &str,
) -> Result<Option<(String, String)>, std::io::Error> {
    let ready_ids = state
        .sets
        .get(ready_key)
        .map(|members| members.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();

    let mut best: Option<(String, String, i64, i64, i64)> = None;
    for task_id in ready_ids {
        let Some(task_json) = state
            .hashes
            .get(tasks_key)
            .and_then(|tasks| tasks.get(task_id.as_str()))
            .cloned()
        else {
            set_remove(state, ready_key, task_id.as_str());
            hash_delete(state, ready_order_key, task_id.as_str());
            continue;
        };

        let Some((priority, depth)) = task_priority_depth(&task_json)? else {
            set_remove(state, ready_key, task_id.as_str());
            hash_delete(state, ready_order_key, task_id.as_str());
            continue;
        };

        let order = state
            .hashes
            .get(ready_order_key)
            .and_then(|values| values.get(task_id.as_str()))
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(i64::MAX);

        let candidate = (task_id.clone(), task_json, priority, depth, order);
        if is_better_ready_candidate(best.as_ref(), &candidate) {
            best = Some(candidate);
        }
    }

    Ok(best.map(|(task_id, task_json, _, _, _)| (task_id, task_json)))
}

fn is_better_ready_candidate(
    current_best: Option<&(String, String, i64, i64, i64)>,
    candidate: &(String, String, i64, i64, i64),
) -> bool {
    let Some(best) = current_best else {
        return true;
    };

    candidate.2 > best.2
        || (candidate.2 == best.2 && candidate.3 < best.3)
        || (candidate.2 == best.2 && candidate.3 == best.3 && candidate.4 < best.4)
        || (candidate.2 == best.2
            && candidate.3 == best.3
            && candidate.4 == best.4
            && candidate.0 < best.0)
}

fn route_task(
    state: &mut TestRedisState,
    ready_key: &str,
    ready_order_key: &str,
    delayed_key: &str,
    sequence_key: &str,
    task_id: &str,
    task_json: &str,
    now: i64,
) -> Result<(), std::io::Error> {
    match task_ready_at(task_json)? {
        Some(ready_at) if ready_at > now => {
            sorted_set_insert(state, delayed_key, task_id, ready_at);
        }
        _ => {
            push_ready(state, ready_key, ready_order_key, sequence_key, task_id)?;
        }
    }
    Ok(())
}

fn task_ready_at(task_json: &str) -> Result<Option<i64>, std::io::Error> {
    let value: Value = serde_json::from_str(task_json).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid task json: {error}"),
        )
    })?;

    Ok(value.get("ready_at").and_then(Value::as_i64))
}

fn task_priority_depth(task_json: &str) -> Result<Option<(i64, i64)>, std::io::Error> {
    let value: Value = serde_json::from_str(task_json).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid task json: {error}"),
        )
    })?;

    let priority = value.get("priority").and_then(Value::as_i64).unwrap_or(0);
    let depth = value.get("depth").and_then(Value::as_u64).unwrap_or(0);
    Ok(Some((priority, i64::try_from(depth).unwrap_or_default())))
}

fn push_ready(
    state: &mut TestRedisState,
    ready_key: &str,
    ready_order_key: &str,
    sequence_key: &str,
    task_id: &str,
) -> Result<(), std::io::Error> {
    let next_ready_order = increment_string_counter(state, sequence_key)?;
    set_insert(state, ready_key, task_id);
    hash_set(
        state,
        ready_order_key,
        task_id,
        next_ready_order.to_string(),
    );
    Ok(())
}

fn increment_string_counter(state: &mut TestRedisState, key: &str) -> Result<i64, std::io::Error> {
    let next = state
        .strings
        .get(key)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
        + 1;
    state.strings.insert(key.to_string(), next.to_string());
    Ok(next)
}

fn task_exists(state: &TestRedisState, tasks_key: &str, task_id: &str) -> bool {
    state
        .hashes
        .get(tasks_key)
        .is_some_and(|tasks| tasks.contains_key(task_id))
}

fn hash_set(state: &mut TestRedisState, key: &str, field: &str, value: String) {
    state
        .hashes
        .entry(key.to_string())
        .or_default()
        .insert(field.to_string(), value);
}

fn hash_delete(state: &mut TestRedisState, key: &str, field: &str) -> bool {
    state
        .hashes
        .get_mut(key)
        .is_some_and(|values| values.remove(field).is_some())
}

fn lease_matches(
    state: &TestRedisState,
    workers_key: &str,
    leases_key: &str,
    task_id: &str,
    worker_id: &str,
    lease_id: &str,
) -> bool {
    let current_worker = state
        .hashes
        .get(workers_key)
        .and_then(|values| values.get(task_id));
    let current_lease = state
        .hashes
        .get(leases_key)
        .and_then(|values| values.get(task_id));

    current_worker.is_some_and(|value| value == worker_id)
        && current_lease.is_some_and(|value| value == lease_id)
}

fn lease_result_code(
    state: &TestRedisState,
    workers_key: &str,
    leases_key: &str,
    task_id: &str,
    worker_id: &str,
    lease_id: &str,
) -> i64 {
    let current_worker = state
        .hashes
        .get(workers_key)
        .and_then(|values| values.get(task_id));
    let current_lease = state
        .hashes
        .get(leases_key)
        .and_then(|values| values.get(task_id));

    match (current_worker, current_lease) {
        (Some(current_worker), Some(_)) if current_worker != worker_id => -1,
        (Some(_), Some(current_lease)) if current_lease != lease_id => -2,
        _ => 0,
    }
}

fn set_insert(state: &mut TestRedisState, key: &str, member: &str) {
    state
        .sets
        .entry(key.to_string())
        .or_default()
        .insert(member.to_string());
}

fn set_remove(state: &mut TestRedisState, key: &str, member: &str) -> bool {
    state
        .sets
        .get_mut(key)
        .is_some_and(|members| members.remove(member))
}

fn sorted_set_insert(state: &mut TestRedisState, key: &str, member: &str, score: i64) {
    state
        .sorted_sets
        .entry(key.to_string())
        .or_default()
        .insert(member.to_string(), score);
}

fn sorted_set_remove(state: &mut TestRedisState, key: &str, member: &str) -> bool {
    state
        .sorted_sets
        .get_mut(key)
        .is_some_and(|members| members.remove(member).is_some())
}

fn is_implicit_client_command(command: &[String]) -> bool {
    matches!(
        command,
        [name, action, ..] if name == "CLIENT" && action == "SETINFO"
    )
}

fn sorted_members(members: Option<&BTreeMap<String, i64>>) -> Vec<(String, i64)> {
    let mut values = members
        .map(|members| {
            members
                .iter()
                .map(|(member, score)| (member.clone(), *score))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    values.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    values
}

fn parse_score_bound(value: &str) -> Result<i64, std::io::Error> {
    match value {
        "-inf" => Ok(i64::MIN),
        "+inf" | "inf" => Ok(i64::MAX),
        _ => value.parse::<i64>().map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid score bound: {error}"),
            )
        }),
    }
}

fn slice_range_entries(
    members: &[(String, i64)],
    start: &str,
    stop: &str,
) -> Result<Vec<(String, i64)>, std::io::Error> {
    let len = i64::try_from(members.len()).unwrap_or_default();
    let start = normalize_index(start.parse::<i64>().map_err(int_error)?, len);
    let stop = normalize_index(stop.parse::<i64>().map_err(int_error)?, len);

    if len == 0 || start >= len || stop < 0 || start > stop {
        return Ok(Vec::new());
    }

    let start = usize::try_from(start.max(0)).unwrap_or_default();
    let stop = usize::try_from(stop.min(len - 1)).unwrap_or_default();
    Ok(members[start..=stop].to_vec())
}

fn normalize_index(index: i64, len: i64) -> i64 {
    if index < 0 { len + index } else { index }
}

fn int_error(error: std::num::ParseIntError) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("invalid integer: {error}"),
    )
}

fn simple_reply(value: &str) -> Vec<u8> {
    format!("+{value}\r\n").into_bytes()
}

fn integer_reply(value: i64) -> Vec<u8> {
    format!(":{value}\r\n").into_bytes()
}

fn bulk_reply(value: Option<String>) -> Vec<u8> {
    match value {
        Some(value) => {
            let mut payload = format!("${}\r\n", value.len()).into_bytes();
            payload.extend_from_slice(value.as_bytes());
            payload.extend_from_slice(b"\r\n");
            payload
        }
        None => b"$-1\r\n".to_vec(),
    }
}

fn array_reply(values: Vec<String>) -> Vec<u8> {
    let mut payload = format!("*{}\r\n", values.len()).into_bytes();
    for value in values {
        payload.extend_from_slice(format!("${}\r\n", value.len()).as_bytes());
        payload.extend_from_slice(value.as_bytes());
        payload.extend_from_slice(b"\r\n");
    }
    payload
}

fn array_reply_optional(values: Vec<Option<String>>) -> Vec<u8> {
    let mut payload = format!("*{}\r\n", values.len()).into_bytes();
    for value in values {
        payload.extend_from_slice(&bulk_reply(value));
    }
    payload
}

fn error_reply(message: &str) -> Vec<u8> {
    format!("-{message}\r\n").into_bytes()
}

async fn read_resp_command(stream: &mut TcpStream) -> Result<Option<Vec<String>>, std::io::Error> {
    let mut prefix = [0_u8; 1];
    match stream.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }

    if prefix[0] != b'*' {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "resp command did not start with array marker",
        ));
    }

    let count = read_resp_line(stream)
        .await?
        .parse::<usize>()
        .map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid array length: {error}"),
            )
        })?;
    let mut command = Vec::with_capacity(count);

    for _ in 0..count {
        let mut bulk_prefix = [0_u8; 1];
        stream.read_exact(&mut bulk_prefix).await?;
        if bulk_prefix[0] != b'$' {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "resp bulk string did not start with $",
            ));
        }

        let length = read_resp_line(stream)
            .await?
            .parse::<usize>()
            .map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid bulk length: {error}"),
                )
            })?;
        let mut bytes = vec![0_u8; length + 2];
        stream.read_exact(&mut bytes).await?;
        if &bytes[length..] != b"\r\n" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bulk string missing CRLF suffix",
            ));
        }
        command.push(
            String::from_utf8(bytes[..length].to_vec()).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("bulk string was not utf-8: {error}"),
                )
            })?,
        );
    }

    Ok(Some(command))
}

async fn read_resp_line(stream: &mut TcpStream) -> Result<String, std::io::Error> {
    let mut bytes = Vec::new();

    loop {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await?;
        if byte[0] == b'\r' {
            let mut line_feed = [0_u8; 1];
            stream.read_exact(&mut line_feed).await?;
            if line_feed[0] != b'\n' {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "line ended without LF after CR",
                ));
            }
            break;
        }
        bytes.push(byte[0]);
    }

    String::from_utf8(bytes).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("reply line was not utf-8: {error}"),
        )
    })
}
