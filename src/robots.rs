use crate::error::SpiderError;
use crate::request::Request;
use reqwest::Url;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Default)]
pub(crate) struct Robot {
    policies: tokio::sync::Mutex<BTreeMap<String, Arc<Policy>>>,
}

impl Robot {
    pub(crate) async fn is_allowed(
        &self,
        request: &Request,
        user_agent: &str,
    ) -> Result<bool, SpiderError> {
        let url = Url::parse(&request.url)
            .map_err(|error| SpiderError::request_build(error.to_string()))?;

        if !matches!(url.scheme(), "http" | "https") {
            return Ok(true);
        }

        let origin = origin_key(&url);
        let path = request_path(&url);

        let policy = {
            let mut policies = self.policies.lock().await;
            if let Some(policy) = policies.get(&origin) {
                policy.clone()
            } else {
                let policy = Arc::new(fetch_policy(&url, request, user_agent).await?);
                policies.insert(origin, policy.clone());
                policy
            }
        };

        Ok(policy.is_allowed(path.as_str(), user_agent))
    }

    #[cfg(test)]
    pub(crate) async fn seed_from_body(&self, url: &str, body: &str) {
        let url = Url::parse(url).unwrap();
        let origin = origin_key(&url);
        let policy = Arc::new(Policy::parse(body));
        self.policies.lock().await.insert(origin, policy);
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
        if rules.groups.is_empty() {
            Self::AllowAll
        } else {
            Self::Parsed(rules)
        }
    }

    fn is_allowed(&self, path: &str, user_agent: &str) -> bool {
        match self {
            Self::AllowAll => true,
            Self::DisallowAll => false,
            Self::Parsed(rules) => rules.is_allowed(path, user_agent),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Rules {
    groups: Vec<Group>,
}

impl Rules {
    fn parse(body: &str) -> Self {
        let mut groups = Vec::new();
        let mut current_agents: Vec<String> = Vec::new();
        let mut current_rules: Vec<Rule> = Vec::new();

        for raw_line in body.lines() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                finalize_group(&mut groups, &mut current_agents, &mut current_rules);
                continue;
            }

            let Some((name, value)) = line.split_once(':') else {
                continue;
            };

            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();

            match name.as_str() {
                "user-agent" => {
                    if !current_agents.is_empty() && !current_rules.is_empty() {
                        finalize_group(&mut groups, &mut current_agents, &mut current_rules);
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
                _ => {}
            }
        }

        finalize_group(&mut groups, &mut current_agents, &mut current_rules);

        Self { groups }
    }

    fn is_allowed(&self, path: &str, user_agent: &str) -> bool {
        let user_agent = user_agent.to_ascii_lowercase();
        let mut best_group_specificity = None::<usize>;
        let mut matching_rules = Vec::new();

        for group in &self.groups {
            let specificity = group.match_specificity(&user_agent);
            let Some(specificity) = specificity else {
                continue;
            };

            match best_group_specificity {
                None => {
                    best_group_specificity = Some(specificity);
                    matching_rules.clear();
                    matching_rules.extend(group.rules.iter());
                }
                Some(current) if specificity > current => {
                    best_group_specificity = Some(specificity);
                    matching_rules.clear();
                    matching_rules.extend(group.rules.iter());
                }
                Some(current) if specificity == current => {
                    matching_rules.extend(group.rules.iter());
                }
                Some(_) => {}
            }
        }

        let mut best_match = None::<(usize, RuleKind)>;
        for rule in matching_rules {
            if !rule.matches(path) {
                continue;
            }

            match best_match {
                None => best_match = Some((rule.path.len(), rule.kind)),
                Some((best_len, best_kind))
                    if rule.path.len() > best_len
                        || (rule.path.len() == best_len
                            && rule.kind == RuleKind::Allow
                            && best_kind == RuleKind::Disallow) =>
                {
                    best_match = Some((rule.path.len(), rule.kind));
                }
                Some(_) => {}
            }
        }

        !matches!(best_match, Some((_, RuleKind::Disallow)))
    }
}

#[derive(Debug, Clone)]
struct Group {
    agents: Vec<String>,
    rules: Vec<Rule>,
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
    path: String,
}

impl Rule {
    fn allow(path: &str) -> Self {
        Self {
            kind: RuleKind::Allow,
            path: normalize_rule_path(path),
        }
    }

    fn disallow(path: &str) -> Self {
        Self {
            kind: RuleKind::Disallow,
            path: normalize_rule_path(path),
        }
    }

    fn matches(&self, path: &str) -> bool {
        match self.kind {
            RuleKind::Allow => {
                if self.path.is_empty() {
                    return false;
                }
                path.starts_with(self.path.as_str())
            }
            RuleKind::Disallow => {
                if self.path.is_empty() {
                    return false;
                }
                path.starts_with(self.path.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleKind {
    Allow,
    Disallow,
}

async fn fetch_policy(
    url: &Url,
    request: &Request,
    user_agent: &str,
) -> Result<Policy, SpiderError> {
    let robots_url = robots_url(url);
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
                "failed to fetch robots.txt, allowing requests for this origin"
            );
            return Ok(Policy::AllowAll);
        }
    };

    match response.status().as_u16() {
        200..=299 => {
            let body = response.text().await.map_err(|error| {
                SpiderError::download(format!("failed to read robots.txt response body: {error}"))
            })?;
            Ok(Policy::parse(body.as_str()))
        }
        401 | 403 => Ok(Policy::DisallowAll),
        404 => Ok(Policy::AllowAll),
        status => {
            tracing::warn!(
                robots_url = robots_url.as_str(),
                status,
                "robots.txt returned a non-success status, allowing requests for this origin"
            );
            Ok(Policy::AllowAll)
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

fn finalize_group(
    groups: &mut Vec<Group>,
    current_agents: &mut Vec<String>,
    current_rules: &mut Vec<Rule>,
) {
    if current_agents.is_empty() {
        current_rules.clear();
        return;
    }

    groups.push(Group {
        agents: std::mem::take(current_agents),
        rules: std::mem::take(current_rules),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn robot_uses_seeded_policy() {
        let robot = Robot::default();
        robot
            .seed_from_body(
                "https://example.com/private/page",
                "User-agent: *\nDisallow: /private\n",
            )
            .await;

        let request = Request::new("https://example.com/private/page");
        assert!(!robot.is_allowed(&request, "kun").await.unwrap());
    }
}
