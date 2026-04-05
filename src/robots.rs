pub mod cache;

use crate::error::SpiderError;
use crate::future::BoxFuture;
use crate::request::Request;
use jiff::SignedDuration;
use jiff::Timestamp;
use regex::Regex as PatternRegex;
use reqwest::Url;
use std::collections::BTreeMap;
use std::sync::Arc;

pub use cache::{Cache, Entry as CacheEntry, Memory as CacheMemory, Policy as CachePolicy};

/// robots.txt policy boundary for the engine.
///
/// Implementations decide whether a request is allowed for a given user-agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Disallow,
    Delay(SignedDuration),
}

pub trait Robot: Send + Sync {
    /// Returns whether the request is allowed to proceed.
    fn is_allowed<'a>(
        &'a self,
        request: &'a Request,
        user_agent: &'a str,
    ) -> BoxFuture<'a, Result<bool, SpiderError>>;

    /// Returns the full robots decision for a request.
    fn check<'a>(
        &'a self,
        request: &'a Request,
        user_agent: &'a str,
    ) -> BoxFuture<'a, Result<Decision, SpiderError>> {
        Box::pin(async move {
            if self.is_allowed(request, user_agent).await? {
                Ok(Decision::Allow)
            } else {
                Ok(Decision::Disallow)
            }
        })
    }

    /// Returns sitemap URLs declared by the current origin's robots.txt.
    fn sitemaps<'a>(
        &'a self,
        _request: &'a Request,
    ) -> BoxFuture<'a, Result<Vec<String>, SpiderError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Default in-memory robots.txt policy implementation.
///
/// This implementation keeps a hot in-process cache per origin and uses a
/// replaceable cache backend to load or save robots policies.
#[derive(Debug)]
pub struct Memory<C = cache::Memory> {
    cache: C,
    cache_ttl: Option<u64>,
    policies: tokio::sync::Mutex<BTreeMap<String, CachedPolicy>>,
    crawl_deadlines: tokio::sync::Mutex<BTreeMap<String, u64>>,
}

#[derive(Debug, Clone)]
struct CachedPolicy {
    fetched_at: u64,
    policy: Arc<Policy>,
}

impl Memory<cache::Memory> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for Memory<cache::Memory> {
    fn default() -> Self {
        Self {
            cache: cache::Memory::default(),
            cache_ttl: Some(86_400_000),
            policies: tokio::sync::Mutex::new(BTreeMap::new()),
            crawl_deadlines: tokio::sync::Mutex::new(BTreeMap::new()),
        }
    }
}

impl<C> Memory<C> {
    pub fn with_cache<C2: Cache>(self, cache: C2) -> Memory<C2> {
        Memory {
            cache,
            cache_ttl: self.cache_ttl,
            policies: self.policies,
            crawl_deadlines: self.crawl_deadlines,
        }
    }

    pub fn with_cache_ttl(mut self, ttl: SignedDuration) -> Self {
        self.cache_ttl = Some(non_negative_milliseconds(ttl));
        self
    }

    pub fn without_cache_ttl(mut self) -> Self {
        self.cache_ttl = None;
        self
    }
}

impl<C: Cache> Robot for Memory<C> {
    fn is_allowed<'a>(
        &'a self,
        request: &'a Request,
        user_agent: &'a str,
    ) -> BoxFuture<'a, Result<bool, SpiderError>> {
        Box::pin(async move {
            let url = Url::parse(&request.url)
                .map_err(|error| SpiderError::request_build(error.to_string()))?;

            if !matches!(url.scheme(), "http" | "https") {
                return Ok(true);
            }

            let path = request_path(&url);
            let policy = self.policy_for(&url, request, user_agent).await?;

            Ok(policy.is_allowed(path.as_str(), user_agent))
        })
    }

    fn check<'a>(
        &'a self,
        request: &'a Request,
        user_agent: &'a str,
    ) -> BoxFuture<'a, Result<Decision, SpiderError>> {
        Box::pin(async move {
            let url = Url::parse(&request.url)
                .map_err(|error| SpiderError::request_build(error.to_string()))?;

            if !matches!(url.scheme(), "http" | "https") {
                return Ok(Decision::Allow);
            }

            let origin = origin_key(&url);
            let path = request_path(&url);
            let policy = self.policy_for(&url, request, user_agent).await?;

            if !policy.is_allowed(path.as_str(), user_agent) {
                return Ok(Decision::Disallow);
            }

            let Some(crawl_delay) = policy.crawl_delay(user_agent) else {
                return Ok(Decision::Allow);
            };

            if !crawl_delay.is_positive() {
                return Ok(Decision::Allow);
            }

            let now = now();
            let crawl_delay = u64::try_from(crawl_delay.as_millis()).unwrap_or_default();
            let mut crawl_deadlines = self.crawl_deadlines.lock().await;
            let next_allowed_at = crawl_deadlines.get(&origin).copied().unwrap_or_default();

            if next_allowed_at > now {
                let remaining = next_allowed_at.saturating_sub(now);
                return Ok(Decision::Delay(duration_from_millis(remaining)));
            }

            crawl_deadlines.insert(origin, now.saturating_add(crawl_delay));
            Ok(Decision::Allow)
        })
    }

    fn sitemaps<'a>(
        &'a self,
        request: &'a Request,
    ) -> BoxFuture<'a, Result<Vec<String>, SpiderError>> {
        Box::pin(async move {
            let url = Url::parse(&request.url)
                .map_err(|error| SpiderError::request_build(error.to_string()))?;

            if !matches!(url.scheme(), "http" | "https") {
                return Ok(Vec::new());
            }

            let policy = self.policy_for(&url, request, "").await?;
            Ok(policy.sitemaps())
        })
    }
}

impl<C: Cache> Memory<C> {
    async fn policy_for(
        &self,
        url: &Url,
        request: &Request,
        user_agent: &str,
    ) -> Result<Arc<Policy>, SpiderError> {
        let origin = origin_key(url);
        let current_time = now();

        if let Some(policy) = self.fresh_hot_policy(origin.as_str(), current_time).await {
            return Ok(policy);
        }

        let cached_entry = self.cache.load(origin.as_str()).await?;
        if let Some(entry) = cached_entry.as_ref()
            && !self.is_stale(entry.fetched_at, current_time)
        {
            let policy = Arc::new(Policy::from_cache_policy(&entry.policy));
            self.remember_policy(origin.as_str(), entry.fetched_at, policy.clone())
                .await;
            return Ok(policy);
        }

        match fetch_policy_entry(url, request, user_agent).await? {
            FetchResult::Cacheable(entry) => {
                let policy = Arc::new(Policy::from_cache_policy(&entry.policy));
                self.cache.save(&entry).await?;
                self.remember_policy(origin.as_str(), entry.fetched_at, policy.clone())
                    .await;
                Ok(policy)
            }
            FetchResult::AllowAllFallback => {
                if let Some(entry) = cached_entry {
                    tracing::warn!(
                        origin = origin.as_str(),
                        "failed to refresh stale robots cache, reusing previous policy"
                    );
                    let policy = Arc::new(Policy::from_cache_policy(&entry.policy));
                    self.remember_policy(origin.as_str(), entry.fetched_at, policy.clone())
                        .await;
                    return Ok(policy);
                }

                tracing::warn!(
                    origin = origin.as_str(),
                    "failed to fetch robots.txt, allowing requests without caching fallback policy"
                );
                Ok(Arc::new(Policy::AllowAll))
            }
        }
    }

    async fn fresh_hot_policy(&self, origin: &str, current_time: u64) -> Option<Arc<Policy>> {
        let policies = self.policies.lock().await;
        let policy = policies.get(origin)?;
        if self.is_stale(policy.fetched_at, current_time) {
            return None;
        }

        Some(policy.policy.clone())
    }

    async fn remember_policy(&self, origin: &str, fetched_at: u64, policy: Arc<Policy>) {
        self.policies
            .lock()
            .await
            .insert(origin.to_string(), CachedPolicy { fetched_at, policy });
    }

    fn is_stale(&self, fetched_at: u64, current_time: u64) -> bool {
        let Some(cache_ttl) = self.cache_ttl else {
            return false;
        };

        current_time.saturating_sub(fetched_at) >= cache_ttl
    }
}

impl Memory<cache::Memory> {
    #[cfg(test)]
    pub(crate) async fn seed_from_body(&self, url: &str, body: &str) {
        self.seed_from_body_at(url, body, now()).await;
    }

    #[cfg(test)]
    pub(crate) async fn seed_from_body_at(&self, url: &str, body: &str, fetched_at: u64) {
        let url = Url::parse(url).unwrap();
        let origin = origin_key(&url);
        let entry = cache::Entry::new(origin, fetched_at, cache::Policy::Body(body.to_string()));
        self.cache.save(&entry).await.unwrap();
        self.policies.lock().await.clear();
    }
}

/// robots policy implementation that always allows requests.
#[derive(Debug, Default, Clone, Copy)]
pub struct Noop;

impl Robot for Noop {
    fn is_allowed<'a>(
        &'a self,
        _request: &'a Request,
        _user_agent: &'a str,
    ) -> BoxFuture<'a, Result<bool, SpiderError>> {
        Box::pin(async { Ok(true) })
    }
}

#[derive(Debug, Clone)]
enum Policy {
    AllowAll,
    DisallowAll,
    Parsed(Rules),
}

impl Policy {
    fn parse(body: &str) -> Self {
        let rules = Rules::parse(body);
        if rules.groups.is_empty() && rules.sitemaps.is_empty() {
            Self::AllowAll
        } else {
            Self::Parsed(rules)
        }
    }

    fn from_cache_policy(policy: &cache::Policy) -> Self {
        match policy {
            cache::Policy::AllowAll => Self::AllowAll,
            cache::Policy::DisallowAll => Self::DisallowAll,
            cache::Policy::Body(body) => Self::parse(body),
        }
    }

    fn is_allowed(&self, path: &str, user_agent: &str) -> bool {
        match self {
            Self::AllowAll => true,
            Self::DisallowAll => false,
            Self::Parsed(rules) => rules.is_allowed(path, user_agent),
        }
    }

    fn crawl_delay(&self, user_agent: &str) -> Option<SignedDuration> {
        match self {
            Self::AllowAll | Self::DisallowAll => None,
            Self::Parsed(rules) => rules.crawl_delay(user_agent),
        }
    }

    fn sitemaps(&self) -> Vec<String> {
        match self {
            Self::AllowAll | Self::DisallowAll => Vec::new(),
            Self::Parsed(rules) => rules.sitemaps.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Rules {
    groups: Vec<Group>,
    sitemaps: Vec<String>,
}

impl Rules {
    fn parse(body: &str) -> Self {
        let mut groups = Vec::new();
        let mut sitemaps = Vec::new();
        let mut current_agents: Vec<String> = Vec::new();
        let mut current_rules: Vec<Rule> = Vec::new();
        let mut current_crawl_delay = None;

        for raw_line in body.lines() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                finalize_group(
                    &mut groups,
                    &mut current_agents,
                    &mut current_rules,
                    &mut current_crawl_delay,
                );
                continue;
            }

            let Some((name, value)) = line.split_once(':') else {
                continue;
            };

            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();

            match name.as_str() {
                "user-agent" => {
                    if !current_agents.is_empty()
                        && (!current_rules.is_empty() || current_crawl_delay.is_some())
                    {
                        finalize_group(
                            &mut groups,
                            &mut current_agents,
                            &mut current_rules,
                            &mut current_crawl_delay,
                        );
                    }
                    current_agents.push(value.to_ascii_lowercase());
                }
                "allow" => {
                    if !current_agents.is_empty() {
                        current_rules.push(Rule::allow(value));
                    }
                }
                "disallow" => {
                    if !current_agents.is_empty() {
                        current_rules.push(Rule::disallow(value));
                    }
                }
                "crawl-delay" => {
                    if !current_agents.is_empty() {
                        current_crawl_delay = parse_crawl_delay(value);
                    }
                }
                "sitemap" => {
                    if !value.is_empty() && !sitemaps.iter().any(|entry| entry == value) {
                        sitemaps.push(value.to_string());
                    }
                }
                _ => {}
            }
        }

        finalize_group(
            &mut groups,
            &mut current_agents,
            &mut current_rules,
            &mut current_crawl_delay,
        );

        Self { groups, sitemaps }
    }

    fn is_allowed(&self, path: &str, user_agent: &str) -> bool {
        let user_agent = user_agent.to_ascii_lowercase();
        let matching_rules = self
            .matching_groups(user_agent.as_str())
            .into_iter()
            .flat_map(|group| group.rules.iter())
            .collect::<Vec<_>>();

        let mut best_match = None::<(usize, RuleKind)>;
        for rule in matching_rules {
            let Some(match_length) = rule.match_length(path) else {
                continue;
            };

            match best_match {
                None => best_match = Some((match_length, rule.kind)),
                Some((best_len, best_kind))
                    if match_length > best_len
                        || (match_length == best_len
                            && rule.kind == RuleKind::Allow
                            && best_kind == RuleKind::Disallow) =>
                {
                    best_match = Some((match_length, rule.kind));
                }
                Some(_) => {}
            }
        }

        !matches!(best_match, Some((_, RuleKind::Disallow)))
    }

    fn crawl_delay(&self, user_agent: &str) -> Option<SignedDuration> {
        self.matching_groups(user_agent.to_ascii_lowercase().as_str())
            .into_iter()
            .filter_map(|group| group.crawl_delay)
            .max()
            .map(duration_from_millis)
    }

    fn matching_groups<'a>(&'a self, user_agent: &str) -> Vec<&'a Group> {
        let mut best_group_specificity = None::<usize>;
        let mut matching_groups = Vec::new();

        for group in &self.groups {
            let Some(specificity) = group.match_specificity(user_agent) else {
                continue;
            };

            match best_group_specificity {
                None => {
                    best_group_specificity = Some(specificity);
                    matching_groups.clear();
                    matching_groups.push(group);
                }
                Some(current) if specificity > current => {
                    best_group_specificity = Some(specificity);
                    matching_groups.clear();
                    matching_groups.push(group);
                }
                Some(current) if specificity == current => {
                    matching_groups.push(group);
                }
                Some(_) => {}
            }
        }

        matching_groups
    }
}

#[derive(Debug, Clone)]
struct Group {
    agents: Vec<String>,
    rules: Vec<Rule>,
    crawl_delay: Option<u64>,
}

impl Group {
    fn match_specificity(&self, user_agent: &str) -> Option<usize> {
        self.agents
            .iter()
            .filter_map(|agent| {
                if agent == "*" {
                    Some(0)
                } else if user_agent.contains(agent) {
                    Some(agent.len())
                } else {
                    None
                }
            })
            .max()
    }
}

#[derive(Debug, Clone)]
struct Rule {
    kind: RuleKind,
    pattern: String,
    match_regex: Option<PatternRegex>,
    specificity_len: usize,
}

impl Rule {
    fn allow(path: &str) -> Self {
        Self::new(RuleKind::Allow, path)
    }

    fn disallow(path: &str) -> Self {
        Self::new(RuleKind::Disallow, path)
    }

    fn new(kind: RuleKind, path: &str) -> Self {
        let pattern = normalize_rule_path(path);
        Self {
            kind,
            match_regex: compile_rule_regex(pattern.as_str()),
            specificity_len: rule_specificity_len(pattern.as_str()),
            pattern,
        }
    }

    fn match_length(&self, path: &str) -> Option<usize> {
        if self.pattern.is_empty() {
            return None;
        }

        if let Some(match_regex) = &self.match_regex {
            return match_regex.is_match(path).then_some(self.specificity_len);
        }

        path.starts_with(self.pattern.as_str())
            .then_some(self.specificity_len)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleKind {
    Allow,
    Disallow,
}

enum FetchResult {
    Cacheable(cache::Entry),
    AllowAllFallback,
}

async fn fetch_policy_entry(
    url: &Url,
    request: &Request,
    user_agent: &str,
) -> Result<FetchResult, SpiderError> {
    let robots_url = robots_url(url);
    let origin = origin_key(url);
    let client = build_client(request)?;
    let mut request_builder = client.get(robots_url.clone());
    if !user_agent.is_empty() {
        request_builder = request_builder.header(reqwest::header::USER_AGENT, user_agent);
    }

    let response = match request_builder.send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                robots_url = robots_url.as_str(),
                error = %error,
                "failed to fetch robots.txt"
            );
            return Ok(FetchResult::AllowAllFallback);
        }
    };

    match response.status().as_u16() {
        200..=299 => {
            let body = match response.text().await {
                Ok(body) => body,
                Err(error) => {
                    tracing::warn!(
                        robots_url = robots_url.as_str(),
                        error = %error,
                        "failed to read robots.txt response body"
                    );
                    return Ok(FetchResult::AllowAllFallback);
                }
            };
            Ok(FetchResult::Cacheable(cache::Entry::new(
                origin,
                now(),
                cache::Policy::Body(body),
            )))
        }
        401 | 403 => Ok(FetchResult::Cacheable(cache::Entry::new(
            origin,
            now(),
            cache::Policy::DisallowAll,
        ))),
        404 => Ok(FetchResult::Cacheable(cache::Entry::new(
            origin,
            now(),
            cache::Policy::AllowAll,
        ))),
        status => {
            tracing::warn!(
                robots_url = robots_url.as_str(),
                status,
                "robots.txt returned a non-success status"
            );
            Ok(FetchResult::AllowAllFallback)
        }
    }
}

fn build_client(request: &Request) -> Result<reqwest::Client, SpiderError> {
    let mut builder = reqwest::Client::builder();

    if let Some(proxy) = &request.proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy.url.as_str())
                .map_err(|error| SpiderError::request_build(error.to_string()))?,
        );
    }

    if let Some(timeout) = request.timeout {
        let timeout = std::time::Duration::try_from(timeout).map_err(|error| {
            SpiderError::request_build(format!("invalid robots.txt request timeout: {error}"))
        })?;
        builder = builder.timeout(timeout);
    }

    builder
        .build()
        .map_err(|error| SpiderError::request_build(error.to_string()))
}

fn robots_url(url: &Url) -> Url {
    let mut robots_url = url.clone();
    robots_url.set_path("/robots.txt");
    robots_url.set_query(None);
    robots_url.set_fragment(None);
    robots_url
}

fn origin_key(url: &Url) -> String {
    let mut origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        origin.push(':');
        origin.push_str(port.to_string().as_str());
    }
    origin
}

fn request_path(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    }
}

fn strip_comment(line: &str) -> &str {
    match line.split_once('#') {
        Some((content, _)) => content,
        None => line,
    }
}

fn normalize_rule_path(path: &str) -> String {
    path.trim().to_string()
}

fn parse_crawl_delay(value: &str) -> Option<u64> {
    let seconds = value.parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }

    Some((seconds * 1000.0).round() as u64)
}

fn compile_rule_regex(pattern: &str) -> Option<PatternRegex> {
    if pattern.is_empty() || (!pattern.contains('*') && !pattern.ends_with('$')) {
        return None;
    }

    let mut regex = String::from("^");
    let mut chars = pattern.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '*' => regex.push_str(".*"),
            '$' if chars.peek().is_none() => regex.push('$'),
            other => regex.push_str(regex::escape(other.to_string().as_str()).as_str()),
        }
    }

    PatternRegex::new(regex.as_str()).ok()
}

fn rule_specificity_len(pattern: &str) -> usize {
    pattern
        .trim_end_matches('$')
        .chars()
        .filter(|ch| *ch != '*')
        .count()
}

fn duration_from_millis(milliseconds: u64) -> SignedDuration {
    SignedDuration::from_millis(i64::try_from(milliseconds).unwrap_or(i64::MAX))
}

fn non_negative_milliseconds(duration: SignedDuration) -> u64 {
    if duration.is_negative() {
        return 0;
    }

    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn now() -> u64 {
    u64::try_from(Timestamp::now().as_millisecond()).unwrap_or_default()
}

fn finalize_group(
    groups: &mut Vec<Group>,
    current_agents: &mut Vec<String>,
    current_rules: &mut Vec<Rule>,
    current_crawl_delay: &mut Option<u64>,
) {
    if current_agents.is_empty() {
        current_rules.clear();
        *current_crawl_delay = None;
        return;
    }

    groups.push(Group {
        agents: std::mem::take(current_agents),
        rules: std::mem::take(current_rules),
        crawl_delay: current_crawl_delay.take(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn rules_use_wildcard_group_when_no_specific_user_agent_matches() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Disallow: /private
            "#,
        );

        assert!(rules.is_allowed("/news", "kun-bot"));
        assert!(!rules.is_allowed("/private/page", "kun-bot"));
    }

    #[test]
    fn rules_prefer_specific_user_agent_group_over_wildcard() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Disallow: /

            User-agent: kun
            Allow: /
            "#,
        );

        assert!(rules.is_allowed("/anything", "kun-bot"));
        assert!(!rules.is_allowed("/anything", "other-bot"));
    }

    #[test]
    fn rules_prefer_longer_match_and_allow_on_same_length() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Disallow: /private
            Allow: /private/public
            "#,
        );

        assert!(!rules.is_allowed("/private/secret", "kun"));
        assert!(rules.is_allowed("/private/public", "kun"));
        assert!(rules.is_allowed("/private/public/page", "kun"));
    }

    #[test]
    fn rules_support_wildcard_and_end_anchor_matching() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Disallow: /search?*
            Disallow: /feed/*.xml.gz$
            Allow: /feed/*.xml$
            "#,
        );

        assert!(!rules.is_allowed("/search?q=rust", "kun"));
        assert!(rules.is_allowed("/feed/daily.xml", "kun"));
        assert!(!rules.is_allowed("/feed/daily.xml.gz", "kun"));
    }

    #[test]
    fn rules_pick_crawl_delay_from_best_matching_group() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Crawl-delay: 2

            User-agent: kun
            Crawl-delay: 0.5
            "#,
        );

        assert_eq!(
            rules.crawl_delay("kun-bot"),
            Some(SignedDuration::from_millis(500))
        );
        assert_eq!(
            rules.crawl_delay("other-bot"),
            Some(SignedDuration::from_secs(2))
        );
    }

    #[test]
    fn rules_collect_sitemaps() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Allow: /
            Sitemap: https://example.com/sitemap.xml
            Sitemap: https://example.com/news.xml
            "#,
        );

        assert_eq!(
            rules.sitemaps,
            vec![
                "https://example.com/sitemap.xml".to_string(),
                "https://example.com/news.xml".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn robot_uses_seeded_policy() {
        let robot = Memory::default();
        robot
            .seed_from_body(
                "https://example.com/private/page",
                "User-agent: *\nDisallow: /private\n",
            )
            .await;

        let request = Request::new("https://example.com/private/page");
        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
    }

    #[tokio::test]
    async fn robot_returns_delay_for_crawl_delay_window() {
        let robot = Memory::default();
        robot
            .seed_from_body(
                "https://example.com/news/1",
                "User-agent: *\nAllow: /\nCrawl-delay: 0.01\n",
            )
            .await;

        let first_request = Request::new("https://example.com/news/1");
        let second_request = Request::new("https://example.com/news/2");

        let first = robot.check(&first_request, "kun").await.unwrap();
        let second = robot.check(&second_request, "kun").await.unwrap();

        assert_eq!(first, Decision::Allow);
        assert!(matches!(second, Decision::Delay(delay) if delay.is_positive()));
    }

    #[tokio::test]
    async fn robot_returns_sitemaps_from_seeded_policy() {
        let robot = Memory::default();
        robot
            .seed_from_body(
                "https://example.com/news/1",
                "User-agent: *\nAllow: /\nSitemap: https://example.com/sitemap.xml\n",
            )
            .await;

        let request = Request::new("https://example.com/news/1");
        let sitemaps = robot.sitemaps(&request).await.unwrap();

        assert_eq!(
            sitemaps,
            vec!["https://example.com/sitemap.xml".to_string()]
        );
    }

    #[tokio::test]
    async fn robot_preserves_sitemaps_when_cached_body_has_no_groups() {
        let robot = Memory::default();
        robot
            .seed_from_body(
                "https://example.com/news/1",
                "Sitemap: https://example.com/sitemap.xml\n",
            )
            .await;

        let request = Request::new("https://example.com/news/1");

        assert!(robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(
            robot.sitemaps(&request).await.unwrap(),
            vec!["https://example.com/sitemap.xml".to_string()]
        );
    }

    #[tokio::test]
    async fn robot_can_use_replaceable_cache_backend() {
        let url = "https://example.com/private/page";
        let origin = origin_key(&Url::parse(url).unwrap());
        let cache = SeededCache::with_entry(cache::Entry::new(
            origin,
            now(),
            cache::Policy::Body("User-agent: *\nDisallow: /private\n".to_string()),
        ));
        let robot = Memory::new().with_cache(cache);

        let request = Request::new(url);
        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
    }

    #[tokio::test]
    async fn robot_saves_fetched_policy_into_cache_backend() {
        let body = "User-agent: *\nDisallow: /private\n";
        let (base_url, server_handle) = spawn_robots_server(body).await;
        let cache = Arc::new(RecordingCache::default());
        let robot = Memory::new().with_cache(cache.clone());
        let request = Request::new(format!("{base_url}/private/page"));

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());

        let saved = cache.saved.lock().await.clone();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved[0],
            cache::Entry::new(
                base_url,
                saved[0].fetched_at,
                cache::Policy::Body(body.to_string()),
            )
        );

        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn robot_refreshes_stale_cached_policy() {
        let body = "User-agent: *\nDisallow: /private\n";
        let (base_url, server_handle) = spawn_robots_server(body).await;
        let stale_at = now().saturating_sub(5_000);
        let cache = SeededCache::with_entry(cache::Entry::new(
            base_url.clone(),
            stale_at,
            cache::Policy::AllowAll,
        ));
        let robot = Memory::new()
            .with_cache(cache)
            .with_cache_ttl(SignedDuration::from_secs(1));
        let request = Request::new(format!("{base_url}/private/page"));

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());

        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn robot_reuses_stale_cached_policy_when_refresh_fails() {
        let stale_at = now().saturating_sub(5_000);
        let cache = SeededCache::with_entry(cache::Entry::new(
            "http://127.0.0.1:9",
            stale_at,
            cache::Policy::Body("User-agent: *\nDisallow: /private\n".to_string()),
        ));
        let robot = Memory::new()
            .with_cache(cache)
            .with_cache_ttl(SignedDuration::from_secs(1));
        let request = Request::new("http://127.0.0.1:9/private/page");

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
    }

    #[tokio::test]
    async fn robot_does_not_refresh_when_cache_refresh_is_disabled() {
        let stale_at = now().saturating_sub(86_400_000 * 2);
        let cache = SeededCache::with_entry(cache::Entry::new(
            "https://example.com",
            stale_at,
            cache::Policy::Body("User-agent: *\nDisallow: /private\n".to_string()),
        ));
        let robot = Memory::new().with_cache(cache).without_cache_ttl();
        let request = Request::new("https://example.com/private/page");

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
    }

    #[derive(Debug, Default)]
    struct SeededCache {
        entries: tokio::sync::Mutex<BTreeMap<String, cache::Entry>>,
    }

    impl SeededCache {
        fn with_entry(entry: cache::Entry) -> Self {
            let mut entries = BTreeMap::new();
            entries.insert(entry.origin.clone(), entry);
            Self {
                entries: tokio::sync::Mutex::new(entries),
            }
        }
    }

    impl Cache for SeededCache {
        fn load<'a>(
            &'a self,
            origin: &'a str,
        ) -> BoxFuture<'a, Result<Option<cache::Entry>, SpiderError>> {
            Box::pin(async move { Ok(self.entries.lock().await.get(origin).cloned()) })
        }

        fn save<'a>(&'a self, entry: &'a cache::Entry) -> BoxFuture<'a, Result<(), SpiderError>> {
            Box::pin(async move {
                self.entries
                    .lock()
                    .await
                    .insert(entry.origin.clone(), entry.clone());
                Ok(())
            })
        }
    }

    #[derive(Debug, Default)]
    struct RecordingCache {
        saved: tokio::sync::Mutex<Vec<cache::Entry>>,
    }

    impl Cache for Arc<RecordingCache> {
        fn load<'a>(
            &'a self,
            _origin: &'a str,
        ) -> BoxFuture<'a, Result<Option<cache::Entry>, SpiderError>> {
            Box::pin(async { Ok(None) })
        }

        fn save<'a>(&'a self, entry: &'a cache::Entry) -> BoxFuture<'a, Result<(), SpiderError>> {
            Box::pin(async move {
                self.saved.lock().await.push(entry.clone());
                Ok(())
            })
        }
    }

    async fn spawn_robots_server(
        body: &str,
    ) -> (String, tokio::task::JoinHandle<Result<(), std::io::Error>>) {
        spawn_robots_server_response(200, "OK", body).await
    }

    async fn spawn_robots_server_response(
        status: u16,
        reason: &str,
        body: &str,
    ) -> (String, tokio::task::JoinHandle<Result<(), std::io::Error>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_string();
        let reason = reason.to_string();

        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await?;
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
            stream.shutdown().await?;
            Ok(())
        });

        (format!("http://{}", address), handle)
    }
}
