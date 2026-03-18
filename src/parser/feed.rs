use quick_xml::Reader;
use quick_xml::events::Event;

/// A single item from an RSS or Atom feed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedItem {
    pub title: Option<String>,
    pub link: Option<String>,
    pub description: Option<String>,
    pub pub_date: Option<String>,
    pub guid: Option<String>,
    pub author: Option<String>,
}

/// Parser for RSS 2.0 and Atom feeds backed by `quick-xml`.
#[derive(Debug, Clone)]
pub struct FeedQuery {
    input: String,
}

impl FeedQuery {
    pub fn new(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
        }
    }

    /// Returns the feed-level title (RSS `<title>` or Atom `<title>` outside entries).
    pub fn title(&self) -> Option<String> {
        parse_feed_title(&self.input)
    }

    /// Returns all items/entries in the feed.
    pub fn items(&self) -> Vec<FeedItem> {
        parse_items(&self.input)
    }

    /// Convenience: collect every non-empty item link.
    pub fn links(&self) -> Vec<String> {
        self.items()
            .into_iter()
            .filter_map(|item| item.link)
            .collect()
    }

    /// Convenience: collect every non-empty item title.
    pub fn titles(&self) -> Vec<String> {
        self.items()
            .into_iter()
            .filter_map(|item| item.title)
            .collect()
    }
}

fn parse_feed_title(input: &str) -> Option<String> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut depth: u32 = 0;
    let mut in_item = false;
    let mut capture = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local = local_name(e.name().as_ref());
                depth += 1;
                if matches!(local.as_str(), "item" | "entry") {
                    in_item = true;
                }
                if !in_item && local == "title" && depth <= 3 {
                    capture = true;
                }
            }
            Ok(Event::Text(ref e)) if capture => {
                if let Ok(text) = e.unescape() {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        return Some(trimmed);
                    }
                }
                capture = false;
            }
            Ok(Event::End(ref e)) => {
                let local = local_name(e.name().as_ref());
                if matches!(local.as_str(), "item" | "entry") {
                    in_item = false;
                }
                if local == "title" {
                    capture = false;
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {
                capture = false;
            }
        }
        buf.clear();
    }
    None
}

fn parse_items(input: &str) -> Vec<FeedItem> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().trim_text(true);

    let mut items: Vec<FeedItem> = Vec::new();
    let mut current: Option<FeedItem> = None;
    let mut buf = Vec::new();
    let mut current_field: Option<&'static str> = None;
    let mut is_atom_link = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "item" | "entry" => {
                        current = Some(FeedItem::default());
                    }
                    "title" if current.is_some() => {
                        current_field = Some("title");
                    }
                    "link" if current.is_some() => {
                        current_field = Some("link");
                        is_atom_link = false;
                    }
                    "description" | "summary" | "content" if current.is_some() => {
                        current_field = Some("description");
                    }
                    "pubDate" | "published" | "updated" if current.is_some() => {
                        current_field = Some("pub_date");
                    }
                    "guid" | "id" if current.is_some() => {
                        current_field = Some("guid");
                    }
                    "author" | "dc:creator" if current.is_some() => {
                        current_field = Some("author");
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = local_name(e.name().as_ref());
                if local == "link" {
                    if let Some(item) = current.as_mut() {
                        if let Some(href) = attr_value(e, b"href") {
                            item.link = Some(href);
                            is_atom_link = true;
                        }
                    }
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Some(field) = current_field {
                    if let Some(item) = current.as_mut() {
                        if let Ok(text) = e.unescape() {
                            let t = text.trim().to_string();
                            if !t.is_empty() && !(field == "link" && is_atom_link) {
                                assign_field(item, field, t);
                            }
                        }
                    }
                }
            }
            Ok(Event::CData(ref e)) => {
                if let Some(field) = current_field {
                    if let Some(item) = current.as_mut() {
                        if let Ok(text) = std::str::from_utf8(e) {
                            let t = text.trim().to_string();
                            if !t.is_empty() {
                                assign_field(item, field, t);
                            }
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let local = local_name(e.name().as_ref());
                if matches!(local.as_str(), "item" | "entry") {
                    if let Some(item) = current.take() {
                        items.push(item);
                    }
                }
                current_field = None;
                is_atom_link = false;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    items
}

fn local_name(name: &[u8]) -> String {
    let s = std::str::from_utf8(name).unwrap_or("");
    s.rsplit_once(':')
        .map(|(_, local)| local)
        .unwrap_or(s)
        .to_string()
}

fn assign_field(item: &mut FeedItem, field: &str, value: String) {
    match field {
        "title" => item.title = Some(value),
        "link" => item.link = Some(value),
        "description" => item.description = Some(value),
        "pub_date" => item.pub_date = Some(value),
        "guid" => item.guid = Some(value),
        "author" => item.author = Some(value),
        _ => {}
    }
}

fn attr_value(e: &quick_xml::events::BytesStart<'_>, attr_name: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == attr_name {
            return attr
                .unescape_value()
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Example Blog</title>
    <item>
      <title>First Post</title>
      <link>https://example.com/post/1</link>
      <description>The first post.</description>
      <pubDate>Mon, 01 Jan 2024 00:00:00 +0000</pubDate>
      <guid>https://example.com/post/1</guid>
    </item>
    <item>
      <title>Second Post</title>
      <link>https://example.com/post/2</link>
      <description><![CDATA[<p>The second post.</p>]]></description>
      <author>alice@example.com</author>
    </item>
  </channel>
</rss>"#;

    const ATOM_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Example Atom</title>
  <entry>
    <title>Atom Entry One</title>
    <link href="https://example.com/atom/1"/>
    <summary>Summary of entry one.</summary>
    <published>2024-01-01T00:00:00Z</published>
    <id>urn:uuid:1</id>
  </entry>
  <entry>
    <title>Atom Entry Two</title>
    <link href="https://example.com/atom/2"/>
    <author><name>Bob</name></author>
  </entry>
</feed>"#;

    #[test]
    fn rss_feed_title() {
        let q = FeedQuery::new(RSS_FEED);
        assert_eq!(q.title().as_deref(), Some("Example Blog"));
    }

    #[test]
    fn rss_feed_items_count() {
        let q = FeedQuery::new(RSS_FEED);
        assert_eq!(q.items().len(), 2);
    }

    #[test]
    fn rss_feed_item_fields() {
        let items = FeedQuery::new(RSS_FEED).items();
        let first = &items[0];

        assert_eq!(first.title.as_deref(), Some("First Post"));
        assert_eq!(first.link.as_deref(), Some("https://example.com/post/1"));
        assert_eq!(first.description.as_deref(), Some("The first post."));
        assert_eq!(first.guid.as_deref(), Some("https://example.com/post/1"));
    }

    #[test]
    fn rss_feed_cdata_description() {
        let items = FeedQuery::new(RSS_FEED).items();
        let second = &items[1];

        assert_eq!(second.description.as_deref(), Some("<p>The second post.</p>"));
        assert_eq!(second.author.as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn rss_feed_links() {
        let links = FeedQuery::new(RSS_FEED).links();
        assert_eq!(
            links,
            vec!["https://example.com/post/1", "https://example.com/post/2"]
        );
    }

    #[test]
    fn atom_feed_title() {
        let q = FeedQuery::new(ATOM_FEED);
        assert_eq!(q.title().as_deref(), Some("Example Atom"));
    }

    #[test]
    fn atom_feed_items_count() {
        let q = FeedQuery::new(ATOM_FEED);
        assert_eq!(q.items().len(), 2);
    }

    #[test]
    fn atom_feed_item_link_from_href_attr() {
        let items = FeedQuery::new(ATOM_FEED).items();
        let first = &items[0];

        assert_eq!(first.title.as_deref(), Some("Atom Entry One"));
        assert_eq!(first.link.as_deref(), Some("https://example.com/atom/1"));
        assert_eq!(first.description.as_deref(), Some("Summary of entry one."));
        assert_eq!(first.guid.as_deref(), Some("urn:uuid:1"));
    }

    #[test]
    fn empty_input_returns_no_items() {
        let q = FeedQuery::new("");
        assert!(q.items().is_empty());
        assert!(q.title().is_none());
    }

    #[test]
    fn feed_titles_convenience() {
        let titles = FeedQuery::new(RSS_FEED).titles();
        assert_eq!(titles, vec!["First Post", "Second Post"]);
    }
}
