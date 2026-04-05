use quick_xml::Reader;
use quick_xml::events::Event;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sitemap {
    pub urls: Vec<String>,
    pub sitemaps: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SitemapQuery {
    input: String,
}

impl SitemapQuery {
    pub fn new(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
        }
    }

    pub fn entries(&self) -> Sitemap {
        parse_sitemap(&self.input)
    }

    pub fn urls(&self) -> Vec<String> {
        self.entries().urls
    }

    pub fn sitemaps(&self) -> Vec<String> {
        self.entries().sitemaps
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocTarget {
    Url,
    Sitemap,
}

fn parse_sitemap(input: &str) -> Sitemap {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(true);

    let mut result = Sitemap::default();
    let mut buf = Vec::new();
    let mut current_parent: Option<LocTarget> = None;
    let mut capture_loc = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref event)) => {
                let local = local_name(event.name().as_ref());
                match local.as_str() {
                    "url" => current_parent = Some(LocTarget::Url),
                    "sitemap" => current_parent = Some(LocTarget::Sitemap),
                    "loc" => capture_loc = current_parent.is_some(),
                    _ => {}
                }
            }
            Ok(Event::Text(ref event)) => {
                if capture_loc && let Some(target) = current_parent {
                    let text = String::from_utf8_lossy(event.as_ref());
                    push_loc(&mut result, target, text.trim());
                }
            }
            Ok(Event::CData(ref event)) => {
                if capture_loc
                    && let Some(target) = current_parent
                    && let Ok(text) = std::str::from_utf8(event)
                {
                    push_loc(&mut result, target, text.trim());
                }
            }
            Ok(Event::End(ref event)) => {
                let local = local_name(event.name().as_ref());
                match local.as_str() {
                    "loc" => capture_loc = false,
                    "url" | "sitemap" => {
                        current_parent = None;
                        capture_loc = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    result
}

fn push_loc(result: &mut Sitemap, target: LocTarget, text: &str) {
    if text.is_empty() {
        return;
    }

    match target {
        LocTarget::Url => result.urls.push(text.to_string()),
        LocTarget::Sitemap => result.sitemaps.push(text.to_string()),
    }
}

fn local_name(name: &[u8]) -> String {
    let name = std::str::from_utf8(name).unwrap_or("");
    name.rsplit_once(':')
        .map(|(_, local)| local)
        .unwrap_or(name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sitemap_query_reads_urlset_urls() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>https://example.com/a</loc>
    <lastmod>2026-04-04</lastmod>
  </url>
  <url>
    <loc><![CDATA[https://example.com/b]]></loc>
  </url>
</urlset>"#;

        let sitemap = SitemapQuery::new(xml).entries();

        assert_eq!(
            sitemap.urls,
            vec![
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string()
            ]
        );
        assert!(sitemap.sitemaps.is_empty());
    }

    #[test]
    fn sitemap_query_reads_sitemap_index_links() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap>
    <loc>https://example.com/news.xml</loc>
  </sitemap>
  <sitemap>
    <loc>https://example.com/archive.xml</loc>
  </sitemap>
</sitemapindex>"#;

        let sitemap = SitemapQuery::new(xml).entries();

        assert!(sitemap.urls.is_empty());
        assert_eq!(
            sitemap.sitemaps,
            vec![
                "https://example.com/news.xml".to_string(),
                "https://example.com/archive.xml".to_string()
            ]
        );
    }
}
