use crate::error::SpiderError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use url::Url;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ErrorContext {
    Engine,
    Scheduler,
}

impl ErrorContext {
    fn error(self, message: impl Into<String>) -> SpiderError {
        match self {
            Self::Engine => SpiderError::engine(message),
            Self::Scheduler => SpiderError::scheduler(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub database: Option<u32>,
}

impl Endpoint {
    pub(crate) fn parse(
        url_text: &str,
        label: &str,
        context: ErrorContext,
    ) -> Result<Self, SpiderError> {
        let url = Url::parse(url_text)
            .map_err(|error| context.error(format!("invalid {label} url `{url_text}`: {error}")))?;

        if url.scheme() != "redis" {
            return Err(context.error(format!("{label} url must use redis:// scheme: {url_text}")));
        }

        let host = url
            .host_str()
            .ok_or_else(|| context.error(format!("{label} url must include a host: {url_text}")))?;
        let port = url.port().unwrap_or(6379);
        let username = if url.username().is_empty() {
            None
        } else {
            Some(url.username().to_string())
        };
        let password = url.password().map(str::to_string);
        let database = parse_database(url.path(), url_text, label, context)?;

        Ok(Self {
            host: host.to_string(),
            port,
            username,
            password,
            database,
        })
    }
}

#[derive(Debug)]
pub(crate) struct Connection {
    stream: TcpStream,
}

impl Connection {
    pub(crate) async fn connect(
        endpoint: &Endpoint,
        label: &str,
        context: ErrorContext,
    ) -> Result<Self, SpiderError> {
        let address = format!("{}:{}", endpoint.host, endpoint.port);
        let stream = TcpStream::connect(&address).await.map_err(|error| {
            context.error(format!(
                "failed to connect to {label} at {address}: {error}"
            ))
        })?;
        let mut connection = Self { stream };

        if let Some(password) = &endpoint.password {
            if let Some(username) = &endpoint.username {
                connection
                    .send_command(
                        &["AUTH".to_string(), username.clone(), password.clone()],
                        label,
                        context,
                    )
                    .await?;
            } else {
                connection
                    .send_command(&["AUTH".to_string(), password.clone()], label, context)
                    .await?;
            }
        }

        if let Some(database) = endpoint.database {
            connection
                .send_command(
                    &["SELECT".to_string(), database.to_string()],
                    label,
                    context,
                )
                .await?;
        }

        Ok(connection)
    }

    pub(crate) async fn send_command(
        &mut self,
        args: &[String],
        label: &str,
        context: ErrorContext,
    ) -> Result<Reply, SpiderError> {
        let payload = build_resp_array(args);
        self.stream
            .write_all(&payload)
            .await
            .map_err(|error| context.error(format!("failed to write {label} command: {error}")))?;
        self.stream
            .flush()
            .await
            .map_err(|error| context.error(format!("failed to flush {label} command: {error}")))?;

        read_resp_reply(&mut self.stream, label, context).await
    }

    pub(crate) async fn close(
        &mut self,
        label: &str,
        context: ErrorContext,
    ) -> Result<(), SpiderError> {
        self.stream
            .shutdown()
            .await
            .map_err(|error| context.error(format!("failed to close {label} connection: {error}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    Simple(String),
    Integer(i64),
    Bulk(Option<String>),
    Array(Option<Vec<Reply>>),
}

impl Reply {
    pub(crate) fn into_integer(
        self,
        label: &str,
        context: ErrorContext,
    ) -> Result<i64, SpiderError> {
        match self {
            Self::Integer(value) => Ok(value),
            other => Err(context.error(format!("{label} returned non-integer reply: {other:?}"))),
        }
    }

    pub(crate) fn into_bulk(
        self,
        label: &str,
        context: ErrorContext,
    ) -> Result<Option<String>, SpiderError> {
        match self {
            Self::Bulk(value) => Ok(value),
            other => Err(context.error(format!("{label} returned non-bulk reply: {other:?}"))),
        }
    }

    pub(crate) fn into_strings(
        self,
        label: &str,
        context: ErrorContext,
    ) -> Result<Vec<String>, SpiderError> {
        let Some(values) = self.into_array(label, context)? else {
            return Ok(Vec::new());
        };

        values
            .into_iter()
            .map(|value| match value {
                Self::Bulk(Some(text)) | Self::Simple(text) => Ok(text),
                other => Err(context.error(format!(
                    "{label} returned array element with unsupported reply type: {other:?}"
                ))),
            })
            .collect()
    }

    fn into_array(
        self,
        label: &str,
        context: ErrorContext,
    ) -> Result<Option<Vec<Reply>>, SpiderError> {
        match self {
            Self::Array(values) => Ok(values),
            other => Err(context.error(format!("{label} returned non-array reply: {other:?}"))),
        }
    }
}

fn parse_database(
    path: &str,
    url_text: &str,
    label: &str,
    context: ErrorContext,
) -> Result<Option<u32>, SpiderError> {
    if path.is_empty() || path == "/" {
        return Ok(None);
    }

    let database = path.trim_start_matches('/');
    if database.is_empty() {
        return Ok(None);
    }

    if database.contains('/') {
        return Err(context.error(format!(
            "{label} url database path must contain exactly one database index: {url_text}"
        )));
    }

    database.parse::<u32>().map(Some).map_err(|error| {
        context.error(format!(
            "{label} url database index must be an unsigned integer: {error}"
        ))
    })
}

fn build_resp_array(args: &[String]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(format!("*{}\r\n", args.len()).as_bytes());

    for arg in args {
        payload.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        payload.extend_from_slice(arg.as_bytes());
        payload.extend_from_slice(b"\r\n");
    }

    payload
}

async fn read_resp_reply(
    stream: &mut TcpStream,
    label: &str,
    context: ErrorContext,
) -> Result<Reply, SpiderError> {
    let prefix = read_resp_byte(stream, label, context).await?;

    match prefix {
        b'+' => Ok(Reply::Simple(read_resp_line(stream, label, context).await?)),
        b':' => {
            let value = read_resp_line(stream, label, context).await?;
            let value = value.parse::<i64>().map_err(|error| {
                context.error(format!("{label} returned invalid integer reply: {error}"))
            })?;
            Ok(Reply::Integer(value))
        }
        b'$' => read_resp_bulk(stream, label, context).await,
        b'*' => read_resp_array(stream, label, context).await,
        b'-' => Err(context.error(format!(
            "{label} command failed: {}",
            read_resp_line(stream, label, context).await?
        ))),
        other => Err(context.error(format!(
            "{label} returned unsupported reply type: {}",
            other as char
        ))),
    }
}

async fn read_resp_bulk(
    stream: &mut TcpStream,
    label: &str,
    context: ErrorContext,
) -> Result<Reply, SpiderError> {
    let length = read_resp_line(stream, label, context).await?;
    let length = length.parse::<isize>().map_err(|error| {
        context.error(format!(
            "{label} returned invalid bulk reply length: {error}"
        ))
    })?;

    if length == -1 {
        return Ok(Reply::Bulk(None));
    }

    if length < 0 {
        return Err(context.error(format!("{label} returned negative bulk reply length")));
    }

    let mut bytes = vec![0_u8; length as usize + 2];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|error| context.error(format!("failed to read {label} bulk reply: {error}")))?;
    let value = String::from_utf8(bytes[..length as usize].to_vec())
        .map_err(|error| context.error(format!("{label} returned non-utf8 bulk reply: {error}")))?;
    Ok(Reply::Bulk(Some(value)))
}

async fn read_resp_array(
    stream: &mut TcpStream,
    label: &str,
    context: ErrorContext,
) -> Result<Reply, SpiderError> {
    let length = read_resp_line(stream, label, context).await?;
    let length = length.parse::<isize>().map_err(|error| {
        context.error(format!(
            "{label} returned invalid array reply length: {error}"
        ))
    })?;

    if length == -1 {
        return Ok(Reply::Array(None));
    }

    if length < 0 {
        return Err(context.error(format!("{label} returned negative array reply length")));
    }

    let mut values = Vec::with_capacity(length as usize);
    for _ in 0..length as usize {
        values.push(read_resp_array_item(stream, label, context).await?);
    }

    Ok(Reply::Array(Some(values)))
}

async fn read_resp_array_item(
    stream: &mut TcpStream,
    label: &str,
    context: ErrorContext,
) -> Result<Reply, SpiderError> {
    let prefix = read_resp_byte(stream, label, context).await?;

    match prefix {
        b'+' => Ok(Reply::Simple(read_resp_line(stream, label, context).await?)),
        b':' => {
            let value = read_resp_line(stream, label, context).await?;
            let value = value.parse::<i64>().map_err(|error| {
                context.error(format!("{label} returned invalid integer reply: {error}"))
            })?;
            Ok(Reply::Integer(value))
        }
        b'$' => read_resp_bulk(stream, label, context).await,
        b'-' => Err(context.error(format!(
            "{label} command failed: {}",
            read_resp_line(stream, label, context).await?
        ))),
        b'*' => Err(context.error(format!(
            "{label} returned nested array replies which are unsupported"
        ))),
        other => Err(context.error(format!(
            "{label} returned unsupported array reply type: {}",
            other as char
        ))),
    }
}

async fn read_resp_byte(
    stream: &mut TcpStream,
    label: &str,
    context: ErrorContext,
) -> Result<u8, SpiderError> {
    let mut byte = [0_u8; 1];
    stream
        .read_exact(&mut byte)
        .await
        .map_err(|error| context.error(format!("failed to read {label} reply prefix: {error}")))?;
    Ok(byte[0])
}

async fn read_resp_line(
    stream: &mut TcpStream,
    label: &str,
    context: ErrorContext,
) -> Result<String, SpiderError> {
    let mut bytes = Vec::new();

    loop {
        let byte = read_resp_byte(stream, label, context).await?;
        if byte == b'\r' {
            let line_feed = read_resp_byte(stream, label, context).await?;
            if line_feed != b'\n' {
                return Err(context.error(format!("{label} reply line ended without LF after CR")));
            }
            break;
        }
        bytes.push(byte);
    }

    String::from_utf8(bytes)
        .map_err(|error| context.error(format!("{label} returned non-utf8 reply line: {error}")))
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::{BTreeMap, BTreeSet};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;

    #[derive(Default)]
    struct TestRedisState {
        strings: BTreeMap<String, String>,
        hashes: BTreeMap<String, BTreeMap<String, String>>,
        sets: BTreeMap<String, BTreeSet<String>>,
        sorted_sets: BTreeMap<String, BTreeMap<String, i64>>,
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
            let mut state = TestRedisState::default();
            let mut commands = Vec::new();

            loop {
                let accept_result =
                    tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                        .await;

                let Ok(accept_result) = accept_result else {
                    break;
                };
                let (mut stream, _) = accept_result?;

                while let Some(command) = read_resp_command(&mut stream).await? {
                    let reply = handle_command(&mut state, &command)?;
                    commands.push(command);
                    stream.write_all(&reply).await?;
                }
            }

            commands_tx.send(commands).ok();
            Ok(())
        });

        (address.to_string(), commands_rx, server_handle)
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
                if command.len() != 4 {
                    return Ok(error_reply("ERR wrong number of arguments for ZRANGE"));
                }
                let members = sorted_members(state.sorted_sets.get(&command[1]));
                let range = slice_range(&members, &command[2], &command[3])?;
                Ok(array_reply(range))
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

    fn slice_range(
        members: &[(String, i64)],
        start: &str,
        stop: &str,
    ) -> Result<Vec<String>, std::io::Error> {
        let len = i64::try_from(members.len()).unwrap_or_default();
        let start = normalize_index(start.parse::<i64>().map_err(int_error)?, len);
        let stop = normalize_index(stop.parse::<i64>().map_err(int_error)?, len);

        if len == 0 || start >= len || stop < 0 || start > stop {
            return Ok(Vec::new());
        }

        let start = usize::try_from(start.max(0)).unwrap_or_default();
        let stop = usize::try_from(stop.min(len - 1)).unwrap_or_default();
        Ok(members[start..=stop]
            .iter()
            .map(|(member, _)| member.clone())
            .collect())
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

    fn error_reply(message: &str) -> Vec<u8> {
        format!("-{message}\r\n").into_bytes()
    }

    async fn read_resp_command(
        stream: &mut TcpStream,
    ) -> Result<Option<Vec<String>>, std::io::Error> {
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
}
