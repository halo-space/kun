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

/// Policy to apply when robots.txt is temporarily unavailable and there is no
/// usable cached policy to fall back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnavailablePolicy {
    #[default]
    AllowAll,
    DisallowAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Site {
    Origin(String),
    Host(String),
    Pattern(String),
}

impl Site {
    pub fn origin(value: impl Into<String>) -> Self {
        let value = value.into();
        Self::Origin(normalize_site_origin(value.as_str()))
    }

    pub fn host(value: impl Into<String>) -> Self {
        let value = value.into();
        Self::Host(normalize_site_host(value.as_str()))
    }

    pub fn pattern(value: impl Into<String>) -> Self {
        let value = value.into();
        Self::Pattern(normalize_site_pattern(value.as_str()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiteAccess {
    AllowAll,
    DisallowAll,
}

#[derive(Debug, Clone, Default)]
pub struct SitePolicy {
    access: Option<SiteAccess>,
    delay: Option<u64>,
    sitemaps: Vec<String>,
    unavailable_policy: Option<UnavailablePolicy>,
}

impl SitePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_access(mut self, access: SiteAccess) -> Self {
        self.access = Some(access);
        self
    }

    pub fn with_delay(mut self, delay: SignedDuration) -> Self {
        self.delay = Some(non_negative_milliseconds(delay));
        self
    }

    pub fn with_sitemap(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        if !url.is_empty() && !self.sitemaps.iter().any(|entry| entry == &url) {
            self.sitemaps.push(url);
        }
        self
    }

    pub fn with_unavailable_policy(mut self, policy: UnavailablePolicy) -> Self {
        self.unavailable_policy = Some(policy);
        self
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
    unavailable_policy: UnavailablePolicy,
    unavailable_retry_delay: Option<u64>,
    site_policies: Vec<SiteRule>,
    policies: tokio::sync::Mutex<BTreeMap<String, CachedPolicy>>,
    request_deadlines: tokio::sync::Mutex<BTreeMap<String, u64>>,
    unavailable_retry_at: tokio::sync::Mutex<BTreeMap<String, u64>>,
}

#[derive(Debug, Clone)]
struct CachedPolicy {
    fetched_at: u64,
    policy: Arc<Policy>,
}

#[derive(Debug, Clone)]
struct SiteRule {
    matcher: SiteMatcher,
    policy: SitePolicy,
}

#[derive(Debug, Clone, Default)]
struct ResolvedSitePolicy {
    access: Option<SiteAccess>,
    delay: Option<u64>,
    sitemaps: Vec<String>,
    unavailable_policy: Option<UnavailablePolicy>,
}

#[derive(Debug, Clone)]
enum SiteMatcher {
    Origin(String),
    Host(String),
    Pattern {
        regex: PatternRegex,
        literal_len: usize,
    },
}

impl SiteMatcher {
    fn matches(&self, url: &Url) -> bool {
        match self {
            Self::Origin(origin) => origin_key(url) == *origin,
            Self::Host(host) => request_host(url).eq_ignore_ascii_case(host),
            Self::Pattern { regex, .. } => {
                let host = request_host(url).to_ascii_lowercase();
                regex.is_match(host.as_str())
            }
        }
    }

    fn priority(&self, index: usize) -> (u8, usize, usize) {
        match self {
            Self::Origin(origin) => (3, origin.len(), index),
            Self::Host(host) => (2, host.len(), index),
            Self::Pattern { literal_len, .. } => (1, *literal_len, index),
        }
    }
}

impl From<Site> for SiteMatcher {
    fn from(site: Site) -> Self {
        match site {
            Site::Origin(origin) => Self::Origin(origin),
            Site::Host(host) => Self::Host(host),
            Site::Pattern(value) => Self::Pattern {
                literal_len: site_pattern_specificity(value.as_str()),
                regex: compile_site_pattern_regex(value.as_str()),
            },
        }
    }
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
            unavailable_policy: UnavailablePolicy::AllowAll,
            unavailable_retry_delay: Some(60_000),
            site_policies: Vec::new(),
            policies: tokio::sync::Mutex::new(BTreeMap::new()),
            request_deadlines: tokio::sync::Mutex::new(BTreeMap::new()),
            unavailable_retry_at: tokio::sync::Mutex::new(BTreeMap::new()),
        }
    }
}

impl<C> Memory<C> {
    pub fn with_cache<C2: Cache>(self, cache: C2) -> Memory<C2> {
        Memory {
            cache,
            cache_ttl: self.cache_ttl,
            unavailable_policy: self.unavailable_policy,
            unavailable_retry_delay: self.unavailable_retry_delay,
            site_policies: self.site_policies,
            policies: self.policies,
            request_deadlines: self.request_deadlines,
            unavailable_retry_at: self.unavailable_retry_at,
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

    /// Overrides how this policy behaves when robots.txt is temporarily
    /// unavailable and there is no usable cached policy for the current
    /// origin.
    pub fn with_unavailable_policy(mut self, policy: UnavailablePolicy) -> Self {
        self.unavailable_policy = policy;
        self
    }

    /// Overrides how long a temporary unavailable robots result should be
    /// reused before trying to fetch robots.txt again.
    pub fn with_unavailable_retry_delay(mut self, delay: SignedDuration) -> Self {
        self.unavailable_retry_delay = Some(non_negative_milliseconds(delay));
        self
    }

    pub fn without_unavailable_retry_delay(mut self) -> Self {
        self.unavailable_retry_delay = None;
        self
    }

    pub fn with_site_policy(mut self, site: Site, policy: SitePolicy) -> Self {
        self.site_policies.push(SiteRule {
            matcher: site.into(),
            policy,
        });
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

            let site_policy = self.site_policy(&url);
            let policy = self
                .policy_for(&url, request, user_agent, &site_policy)
                .await?;
            Ok(match site_policy.access {
                Some(SiteAccess::AllowAll) => true,
                Some(SiteAccess::DisallowAll) => false,
                None => policy.is_allowed(&url, user_agent),
            })
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
            let site_policy = self.site_policy(&url);
            let policy = self
                .policy_for(&url, request, user_agent, &site_policy)
                .await?;

            if matches!(site_policy.access, Some(SiteAccess::DisallowAll)) {
                return Ok(Decision::Disallow);
            }

            if !matches!(site_policy.access, Some(SiteAccess::AllowAll))
                && !policy.is_allowed(&url, user_agent)
            {
                return Ok(Decision::Disallow);
            }

            let required_delay = max_delay(
                policy.required_delay(user_agent),
                site_policy.delay.map(duration_from_millis),
            );

            let Some(required_delay) = required_delay else {
                return Ok(Decision::Allow);
            };

            if !required_delay.is_positive() {
                return Ok(Decision::Allow);
            }

            let now = now();
            let required_delay = u64::try_from(required_delay.as_millis()).unwrap_or_default();
            let mut request_deadlines = self.request_deadlines.lock().await;
            let next_allowed_at = request_deadlines.get(&origin).copied().unwrap_or_default();

            if next_allowed_at > now {
                let remaining = next_allowed_at.saturating_sub(now);
                return Ok(Decision::Delay(duration_from_millis(remaining)));
            }

            request_deadlines.insert(origin, now.saturating_add(required_delay));
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

            let site_policy = self.site_policy(&url);
            let policy = self.policy_for(&url, request, "", &site_policy).await?;
            Ok(merge_sitemaps(
                policy.sitemaps(),
                site_policy.sitemaps.as_slice(),
            ))
        })
    }
}

impl<C: Cache> Memory<C> {
    async fn policy_for(
        &self,
        url: &Url,
        request: &Request,
        user_agent: &str,
        site_policy: &ResolvedSitePolicy,
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

        if self
            .is_unavailable_retry_pending(origin.as_str(), current_time)
            .await
        {
            if let Some(entry) = cached_entry.as_ref() {
                let policy = Arc::new(Policy::from_cache_policy(&entry.policy));
                self.remember_policy(origin.as_str(), entry.fetched_at, policy.clone())
                    .await;
                return Ok(policy);
            }

            let unavailable_policy = site_policy
                .unavailable_policy
                .unwrap_or(self.unavailable_policy);
            return Ok(Arc::new(unavailable_policy.policy()));
        }

        match fetch_policy_entry(url, request, user_agent).await? {
            FetchResult::Cacheable(entry) => {
                let policy = Arc::new(Policy::from_cache_policy(&entry.policy));
                self.clear_unavailable_retry(origin.as_str()).await;
                self.cache.save(&entry).await?;
                self.remember_policy(origin.as_str(), entry.fetched_at, policy.clone())
                    .await;
                Ok(policy)
            }
            FetchResult::Unavailable => {
                if let Some(entry) = cached_entry {
                    self.remember_unavailable_retry(origin.as_str(), current_time)
                        .await;
                    crate::trace::warn(
                        "robots.cache.refresh_failed_reuse",
                        vec![crate::trace::prop("origin", origin.as_str())],
                    );
                    let policy = Arc::new(Policy::from_cache_policy(&entry.policy));
                    self.remember_policy(origin.as_str(), entry.fetched_at, policy.clone())
                        .await;
                    return Ok(policy);
                }

                self.remember_unavailable_retry(origin.as_str(), current_time)
                    .await;
                let unavailable_policy = site_policy
                    .unavailable_policy
                    .unwrap_or(self.unavailable_policy);
                crate::trace::warn(
                    "robots.fetch_unavailable_policy_applied",
                    vec![
                        crate::trace::prop("origin", origin.as_str()),
                        crate::trace::prop("unavailable_policy", format!("{unavailable_policy:?}")),
                    ],
                );
                Ok(Arc::new(unavailable_policy.policy()))
            }
        }
    }

    fn site_policy(&self, url: &Url) -> ResolvedSitePolicy {
        let mut resolved = ResolvedSitePolicy::default();
        let mut access = None;
        let mut unavailable_policy = None;

        for (index, rule) in self.site_policies.iter().enumerate() {
            if !rule.matcher.matches(url) {
                continue;
            }

            let priority = rule.matcher.priority(index);

            if let Some(candidate) = rule.policy.access
                && access
                    .as_ref()
                    .map(|(current, _)| priority > *current)
                    .unwrap_or(true)
            {
                access = Some((priority, candidate));
            }

            resolved.delay = max_millis(resolved.delay, rule.policy.delay);
            resolved.sitemaps = merge_sitemaps(resolved.sitemaps, rule.policy.sitemaps.as_slice());

            if let Some(candidate) = rule.policy.unavailable_policy
                && unavailable_policy
                    .as_ref()
                    .map(|(current, _)| priority > *current)
                    .unwrap_or(true)
            {
                unavailable_policy = Some((priority, candidate));
            }
        }

        resolved.access = access.map(|(_, value)| value);
        resolved.unavailable_policy = unavailable_policy.map(|(_, value)| value);
        resolved
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

    async fn is_unavailable_retry_pending(&self, origin: &str, current_time: u64) -> bool {
        let Some(_) = self.unavailable_retry_delay else {
            return false;
        };

        let mut unavailable_retry_at = self.unavailable_retry_at.lock().await;
        let Some(retry_at) = unavailable_retry_at.get(origin).copied() else {
            return false;
        };

        if retry_at > current_time {
            return true;
        }

        unavailable_retry_at.remove(origin);
        false
    }

    async fn remember_unavailable_retry(&self, origin: &str, current_time: u64) {
        let Some(unavailable_retry_delay) = self.unavailable_retry_delay else {
            return;
        };

        self.unavailable_retry_at.lock().await.insert(
            origin.to_string(),
            current_time.saturating_add(unavailable_retry_delay),
        );
    }

    async fn clear_unavailable_retry(&self, origin: &str) {
        self.unavailable_retry_at.lock().await.remove(origin);
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
    fn allow_all() -> Self {
        Self::AllowAll
    }

    fn disallow_all() -> Self {
        Self::DisallowAll
    }

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

    fn is_allowed(&self, url: &Url, user_agent: &str) -> bool {
        match self {
            Self::AllowAll => true,
            Self::DisallowAll => false,
            Self::Parsed(rules) => rules.is_allowed(url, user_agent),
        }
    }

    fn required_delay(&self, user_agent: &str) -> Option<SignedDuration> {
        match self {
            Self::AllowAll | Self::DisallowAll => None,
            Self::Parsed(rules) => rules.required_delay(user_agent),
        }
    }

    fn sitemaps(&self) -> Vec<String> {
        match self {
            Self::AllowAll | Self::DisallowAll => Vec::new(),
            Self::Parsed(rules) => rules.sitemaps.clone(),
        }
    }
}

impl UnavailablePolicy {
    fn policy(self) -> Policy {
        match self {
            Self::AllowAll => Policy::allow_all(),
            Self::DisallowAll => Policy::disallow_all(),
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
        let mut current_request_interval = None;

        for raw_line in body.lines() {
            let line = strip_utf8_bom(strip_comment(raw_line)).trim();
            if line.is_empty() {
                finalize_group(
                    &mut groups,
                    &mut current_agents,
                    &mut current_rules,
                    &mut current_crawl_delay,
                    &mut current_request_interval,
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
                            &mut current_request_interval,
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
                "request-rate" => {
                    if !current_agents.is_empty() {
                        current_request_interval = parse_request_rate(value);
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
            &mut current_request_interval,
        );

        Self { groups, sitemaps }
    }

    fn is_allowed(&self, url: &Url, user_agent: &str) -> bool {
        let user_agent = user_agent.to_ascii_lowercase();
        let path = request_path(url);
        let matching_rules = self
            .matching_groups(user_agent.as_str())
            .into_iter()
            .flat_map(|group| group.rules.iter())
            .collect::<Vec<_>>();

        let mut best_match = None::<(usize, RuleKind)>;
        for rule in matching_rules {
            let Some(match_length) = rule.match_length(url, path.as_str()) else {
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

    #[cfg(test)]
    fn is_allowed_path(&self, path: &str, user_agent: &str) -> bool {
        let url = Url::parse(format!("https://example.com{path}").as_str()).unwrap();
        self.is_allowed(&url, user_agent)
    }

    fn required_delay(&self, user_agent: &str) -> Option<SignedDuration> {
        self.matching_groups(user_agent.to_ascii_lowercase().as_str())
            .into_iter()
            .filter_map(Group::required_delay)
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
    request_interval: Option<u64>,
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

    fn required_delay(&self) -> Option<u64> {
        match (self.crawl_delay, self.request_interval) {
            (Some(crawl_delay), Some(request_interval)) => Some(crawl_delay.max(request_interval)),
            (Some(crawl_delay), None) => Some(crawl_delay),
            (None, Some(request_interval)) => Some(request_interval),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Rule {
    kind: RuleKind,
    host: Option<String>,
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
        let (host, pattern) = normalize_rule_target(path);
        Self {
            kind,
            host,
            match_regex: compile_rule_regex(pattern.as_str()),
            specificity_len: rule_specificity_len(pattern.as_str()),
            pattern,
        }
    }

    fn match_length(&self, url: &Url, path: &str) -> Option<usize> {
        if self.pattern.is_empty() {
            return None;
        }

        if let Some(host) = &self.host
            && !request_host(url).eq_ignore_ascii_case(host.as_str())
        {
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
    Unavailable,
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
            crate::trace::warn(
                "robots.fetch_failed",
                vec![
                    crate::trace::prop("robots_url", robots_url.as_str()),
                    crate::trace::prop("error", error),
                ],
            );
            return Ok(FetchResult::Unavailable);
        }
    };

    match response.status().as_u16() {
        200..=299 => {
            let body = match response.text().await {
                Ok(body) => body,
                Err(error) => {
                    crate::trace::warn(
                        "robots.body_read_failed",
                        vec![
                            crate::trace::prop("robots_url", robots_url.as_str()),
                            crate::trace::prop("error", error),
                        ],
                    );
                    return Ok(FetchResult::Unavailable);
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
            crate::trace::warn(
                "robots.unsuccessful_status",
                vec![
                    crate::trace::prop("robots_url", robots_url.as_str()),
                    crate::trace::prop("status", status),
                ],
            );
            Ok(FetchResult::Unavailable)
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

fn normalize_site_origin(value: &str) -> String {
    let trimmed = value.trim();
    if let Ok(url) = Url::parse(trimmed) {
        return origin_key(&url);
    }

    trimmed.trim_end_matches('/').to_ascii_lowercase()
}

fn normalize_site_host(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Ok(url) = Url::parse(trimmed) {
        return request_host(&url).to_ascii_lowercase();
    }

    if trimmed.starts_with("//")
        && let Ok(url) = Url::parse(format!("https:{trimmed}").as_str())
    {
        return request_host(&url).to_ascii_lowercase();
    }

    if !trimmed.contains("://")
        && let Ok(url) = Url::parse(format!("https://{trimmed}").as_str())
    {
        return request_host(&url).to_ascii_lowercase();
    }

    trimmed
        .trim_end_matches('/')
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn normalize_site_pattern(value: &str) -> String {
    value
        .trim()
        .trim_end_matches('/')
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn request_path(url: &Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    }
}

fn request_host(url: &Url) -> &str {
    url.host_str().unwrap_or_default()
}

fn strip_comment(line: &str) -> &str {
    match line.split_once('#') {
        Some((content, _)) => content,
        None => line,
    }
}

fn strip_utf8_bom(line: &str) -> &str {
    line.strip_prefix('\u{feff}').unwrap_or(line)
}

fn normalize_rule_target(value: &str) -> (Option<String>, String) {
    let value = value.trim();
    if value.is_empty() {
        return (None, String::new());
    }

    if let Ok(url) = Url::parse(value) {
        return (
            Some(request_host(&url).to_ascii_lowercase()),
            request_path(&url),
        );
    }

    if value.starts_with("//")
        && let Ok(url) = Url::parse(format!("https:{value}").as_str())
    {
        return (
            Some(request_host(&url).to_ascii_lowercase()),
            request_path(&url),
        );
    }

    (None, value.to_string())
}

fn parse_crawl_delay(value: &str) -> Option<u64> {
    let seconds = value.parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }

    Some((seconds * 1000.0).round() as u64)
}

fn parse_request_rate(value: &str) -> Option<u64> {
    let (requests, window_seconds) = value.split_once('/')?;
    let requests = requests.trim().parse::<u64>().ok()?;
    let window_seconds = window_seconds.trim().parse::<f64>().ok()?;

    if requests == 0 || !window_seconds.is_finite() || window_seconds < 0.0 {
        return None;
    }

    Some(((window_seconds * 1000.0) / requests as f64).ceil() as u64)
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

fn compile_site_pattern_regex(pattern: &str) -> PatternRegex {
    let mut regex = String::from("^");

    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            other => regex.push_str(regex::escape(other.to_string().as_str()).as_str()),
        }
    }

    regex.push('$');
    PatternRegex::new(regex.as_str()).expect("escaped site pattern must compile")
}

fn site_pattern_specificity(pattern: &str) -> usize {
    pattern.chars().filter(|ch| *ch != '*').count()
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

fn max_delay(
    left: Option<SignedDuration>,
    right: Option<SignedDuration>,
) -> Option<SignedDuration> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn max_millis(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn merge_sitemaps(mut current: Vec<String>, extras: &[String]) -> Vec<String> {
    for sitemap in extras {
        if !current.iter().any(|entry| entry == sitemap) {
            current.push(sitemap.clone());
        }
    }

    current
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
    current_request_interval: &mut Option<u64>,
) {
    if current_agents.is_empty() {
        current_rules.clear();
        *current_crawl_delay = None;
        *current_request_interval = None;
        return;
    }

    groups.push(Group {
        agents: std::mem::take(current_agents),
        rules: std::mem::take(current_rules),
        crawl_delay: current_crawl_delay.take(),
        request_interval: current_request_interval.take(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::{Duration, sleep, timeout};

    #[test]
    fn rules_use_wildcard_group_when_no_specific_user_agent_matches() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Disallow: /private
            "#,
        );

        assert!(rules.is_allowed_path("/news", "kun-bot"));
        assert!(!rules.is_allowed_path("/private/page", "kun-bot"));
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

        assert!(rules.is_allowed_path("/anything", "kun-bot"));
        assert!(!rules.is_allowed_path("/anything", "other-bot"));
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

        assert!(!rules.is_allowed_path("/private/secret", "kun"));
        assert!(rules.is_allowed_path("/private/public", "kun"));
        assert!(rules.is_allowed_path("/private/public/page", "kun"));
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

        assert!(!rules.is_allowed_path("/search?q=rust", "kun"));
        assert!(rules.is_allowed_path("/feed/daily.xml", "kun"));
        assert!(!rules.is_allowed_path("/feed/daily.xml.gz", "kun"));
    }

    #[test]
    fn rules_strip_utf8_bom_before_parsing_directives() {
        let rules = Rules::parse("\u{feff}User-agent: *\nDisallow: /private\nAllow: /public\n");

        assert!(!rules.is_allowed_path("/private", "kun"));
        assert!(rules.is_allowed_path("/public", "kun"));
    }

    #[test]
    fn rules_support_absolute_url_targets_for_same_host() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Disallow: https://example.com/private
            Allow: https://example.com/private/public
            "#,
        );

        let private_url = Url::parse("https://example.com/private/report").unwrap();
        let public_url = Url::parse("https://example.com/private/public/page").unwrap();
        let other_host_url = Url::parse("https://other.example.com/private/report").unwrap();

        assert!(!rules.is_allowed(&private_url, "kun"));
        assert!(rules.is_allowed(&public_url, "kun"));
        assert!(rules.is_allowed(&other_host_url, "kun"));
    }

    #[test]
    fn rules_support_protocol_relative_targets_for_same_host() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Disallow: //example.com/search?*
            "#,
        );

        let blocked_url = Url::parse("https://example.com/search?q=rust").unwrap();
        let allowed_url = Url::parse("https://example.com/news").unwrap();

        assert!(!rules.is_allowed(&blocked_url, "kun"));
        assert!(rules.is_allowed(&allowed_url, "kun"));
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
            rules.required_delay("kun-bot"),
            Some(SignedDuration::from_millis(500))
        );
        assert_eq!(
            rules.required_delay("other-bot"),
            Some(SignedDuration::from_secs(2))
        );
    }

    #[test]
    fn rules_support_request_rate_as_even_spacing_delay() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Request-rate: 3 / 20
            "#,
        );

        assert_eq!(
            rules.required_delay("kun-bot"),
            Some(SignedDuration::from_millis(6667))
        );
    }

    #[test]
    fn rules_use_stricter_delay_when_crawl_delay_and_request_rate_both_exist() {
        let rules = Rules::parse(
            r#"
            User-agent: *
            Crawl-delay: 1
            Request-rate: 3/1
            "#,
        );

        assert_eq!(
            rules.required_delay("kun-bot"),
            Some(SignedDuration::from_secs(1))
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
    async fn robot_returns_delay_for_request_rate_window() {
        let robot = Memory::default();
        robot
            .seed_from_body(
                "https://example.com/news/1",
                "User-agent: *\nAllow: /\nRequest-rate: 2/1\n",
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
    async fn robot_site_policy_origin_can_force_disallow() {
        let robot = Memory::new().with_site_policy(
            Site::origin("https://example.com/news"),
            SitePolicy::new().with_access(SiteAccess::DisallowAll),
        );
        robot
            .seed_from_body("https://example.com/news/1", "User-agent: *\nAllow: /\n")
            .await;

        let request = Request::new("https://example.com/news/1");

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(
            robot.check(&request, "kun").await.unwrap(),
            Decision::Disallow
        );
    }

    #[tokio::test]
    async fn robot_site_policy_host_can_force_allow() {
        let robot = Memory::new().with_site_policy(
            Site::host("example.com"),
            SitePolicy::new().with_access(SiteAccess::AllowAll),
        );
        robot
            .seed_from_body(
                "https://example.com/private/page",
                "User-agent: *\nDisallow: /private\n",
            )
            .await;

        let request = Request::new("https://example.com/private/page");

        assert!(robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(robot.check(&request, "kun").await.unwrap(), Decision::Allow);
    }

    #[tokio::test]
    async fn robot_site_policy_pattern_can_force_disallow() {
        let robot = Memory::new().with_site_policy(
            Site::pattern("*.example.com"),
            SitePolicy::new().with_access(SiteAccess::DisallowAll),
        );
        robot
            .seed_from_body(
                "https://news.example.com/news/1",
                "User-agent: *\nAllow: /\n",
            )
            .await;

        let request = Request::new("https://news.example.com/news/1");

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(
            robot.check(&request, "kun").await.unwrap(),
            Decision::Disallow
        );
    }

    #[tokio::test]
    async fn robot_site_policy_uses_strictest_delay_across_matches() {
        let robot = Memory::new()
            .with_site_policy(
                Site::pattern("*.example.com"),
                SitePolicy::new().with_delay(SignedDuration::from_millis(20)),
            )
            .with_site_policy(
                Site::host("news.example.com"),
                SitePolicy::new().with_delay(SignedDuration::from_millis(40)),
            );
        robot
            .seed_from_body(
                "https://news.example.com/news/1",
                "User-agent: *\nAllow: /\nCrawl-delay: 0.01\n",
            )
            .await;

        let first_request = Request::new("https://news.example.com/news/1");
        let second_request = Request::new("https://news.example.com/news/2");

        assert_eq!(
            robot.check(&first_request, "kun").await.unwrap(),
            Decision::Allow
        );
        let second = robot.check(&second_request, "kun").await.unwrap();

        assert!(matches!(
            second,
            Decision::Delay(delay) if delay >= SignedDuration::from_millis(30)
        ));
    }

    #[tokio::test]
    async fn robot_site_policy_unions_sitemaps_across_matches() {
        let robot = Memory::new()
            .with_site_policy(
                Site::pattern("*.example.com"),
                SitePolicy::new().with_sitemap("https://news.example.com/network.xml"),
            )
            .with_site_policy(
                Site::host("news.example.com"),
                SitePolicy::new()
                    .with_sitemap("https://news.example.com/custom.xml")
                    .with_sitemap("https://news.example.com/news.xml"),
            );
        robot
            .seed_from_body(
                "https://news.example.com/news/1",
                "User-agent: *\nAllow: /\nSitemap: https://news.example.com/news.xml\n",
            )
            .await;

        let request = Request::new("https://news.example.com/news/1");

        assert_eq!(
            robot.sitemaps(&request).await.unwrap(),
            vec![
                "https://news.example.com/news.xml".to_string(),
                "https://news.example.com/network.xml".to_string(),
                "https://news.example.com/custom.xml".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn robot_site_policy_prefers_more_specific_access() {
        let robot = Memory::new()
            .with_site_policy(
                Site::pattern("*.example.com"),
                SitePolicy::new().with_access(SiteAccess::AllowAll),
            )
            .with_site_policy(
                Site::host("news.example.com"),
                SitePolicy::new().with_access(SiteAccess::DisallowAll),
            );
        robot
            .seed_from_body(
                "https://news.example.com/news/1",
                "User-agent: *\nAllow: /\n",
            )
            .await;

        let request = Request::new("https://news.example.com/news/1");

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(
            robot.check(&request, "kun").await.unwrap(),
            Decision::Disallow
        );
    }

    #[tokio::test]
    async fn robot_site_policy_prefers_later_rule_when_specificity_ties() {
        let robot = Memory::new()
            .with_site_policy(
                Site::host("example.com"),
                SitePolicy::new().with_access(SiteAccess::AllowAll),
            )
            .with_site_policy(
                Site::host("example.com"),
                SitePolicy::new().with_access(SiteAccess::DisallowAll),
            );
        robot
            .seed_from_body("https://example.com/news/1", "User-agent: *\nAllow: /\n")
            .await;

        let request = Request::new("https://example.com/news/1");

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(
            robot.check(&request, "kun").await.unwrap(),
            Decision::Disallow
        );
    }

    #[tokio::test]
    async fn robot_site_policy_prefers_more_specific_unavailable_policy() {
        let robot = Memory::new()
            .with_site_policy(
                Site::pattern("127.*"),
                SitePolicy::new().with_unavailable_policy(UnavailablePolicy::AllowAll),
            )
            .with_site_policy(
                Site::origin("http://127.0.0.1:9/private/page"),
                SitePolicy::new().with_unavailable_policy(UnavailablePolicy::DisallowAll),
            );
        let request = Request::new("http://127.0.0.1:9/private/page");

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(
            robot.check(&request, "kun").await.unwrap(),
            Decision::Disallow
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
    async fn robot_can_disallow_when_robots_is_unavailable() {
        let robot = Memory::new().with_unavailable_policy(UnavailablePolicy::DisallowAll);
        let request = Request::new("http://127.0.0.1:9/private/page");

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(
            robot.check(&request, "kun").await.unwrap(),
            Decision::Disallow
        );
    }

    #[tokio::test]
    async fn robot_reuses_stale_cached_policy_before_unavailable_policy() {
        let stale_at = now().saturating_sub(5_000);
        let cache = SeededCache::with_entry(cache::Entry::new(
            "http://127.0.0.1:9",
            stale_at,
            cache::Policy::Body("User-agent: *\nAllow: /\n".to_string()),
        ));
        let robot = Memory::new()
            .with_cache(cache)
            .with_cache_ttl(SignedDuration::from_secs(1))
            .with_unavailable_policy(UnavailablePolicy::DisallowAll);
        let request = Request::new("http://127.0.0.1:9/private/page");

        assert!(robot.is_allowed(&request, "kun").await.unwrap());
    }

    #[tokio::test]
    async fn robot_reuses_unavailable_policy_until_retry_delay_expires() {
        let (base_url, request_count, server_handle) =
            spawn_counting_robots_server_response(500, "Internal Server Error", "").await;
        let robot = Memory::new().with_unavailable_retry_delay(SignedDuration::from_secs(1));
        let request = Request::new(format!("{base_url}/news"));

        assert!(robot.is_allowed(&request, "kun").await.unwrap());
        assert!(robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(request_count.load(Ordering::SeqCst), 1);

        sleep(Duration::from_millis(1_100)).await;

        assert!(robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(request_count.load(Ordering::SeqCst), 2);

        server_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn robot_reuses_stale_cache_until_unavailable_retry_delay_expires() {
        let (base_url, request_count, server_handle) =
            spawn_counting_robots_server_response(500, "Internal Server Error", "").await;
        let stale_at = now().saturating_sub(5_000);
        let cache = SeededCache::with_entry(cache::Entry::new(
            base_url.clone(),
            stale_at,
            cache::Policy::Body("User-agent: *\nDisallow: /private\n".to_string()),
        ));
        let robot = Memory::new()
            .with_cache(cache)
            .with_cache_ttl(SignedDuration::from_secs(1))
            .with_unavailable_retry_delay(SignedDuration::from_secs(1));
        let request = Request::new(format!("{base_url}/private/page"));

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(request_count.load(Ordering::SeqCst), 1);

        sleep(Duration::from_millis(1_100)).await;

        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
        assert_eq!(request_count.load(Ordering::SeqCst), 2);

        server_handle.await.unwrap().unwrap();
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

    async fn spawn_counting_robots_server_response(
        status: u16,
        reason: &str,
        body: &str,
    ) -> (
        String,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<Result<(), std::io::Error>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let request_count = Arc::new(AtomicUsize::new(0));
        let body = body.to_string();
        let reason = reason.to_string();
        let request_count_for_server = request_count.clone();

        let handle = tokio::spawn(async move {
            loop {
                let accept_result = timeout(Duration::from_secs(3), listener.accept()).await;
                let Ok(accept_result) = accept_result else {
                    break Ok(());
                };
                let (mut stream, _) = accept_result?;
                request_count_for_server.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer).await?;
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await?;
                stream.shutdown().await?;
            }
        });

        (format!("http://{}", address), request_count, handle)
    }
}
