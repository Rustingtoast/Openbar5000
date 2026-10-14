//! Artist information for the BIO panel: MusicBrainz identifies the artist
//! (type, country, active years, genres) and, optionally, Wikipedia adds a
//! biography and photo via the artist's Wikidata entry. Looked up only when
//! the user asks (the BIO tab's button), off the UI thread, and cached by
//! the caller (see `store`), so each artist is fetched once.
//!
//! The HTTP calls are thin; everything that interprets a response is a
//! pure function, tested below against realistic responses.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

/// MusicBrainz requires an identifying User-Agent and at most ~1 request per
/// second: https://musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting
const USER_AGENT: &str = concat!(
    "Openbar5000/",
    env!("CARGO_PKG_VERSION"),
    " ( https://github.com/Rustingtoast/Openbar5000 )"
);
const MUSICBRAINZ_DELAY: Duration = Duration::from_millis(1100);
/// Attempts per request when a service answers "busy" (503 / 429).
const ATTEMPTS: u32 = 3;
/// Longest `Retry-After` honoured before giving up instead.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);

/// When the next MusicBrainz request may go out. Shared by every lookup
/// thread: several lookups can run at once (e.g. the BIO tab open while
/// tracks change), and MusicBrainz answers 503 to more than about one
/// request per second from one address.
static MUSICBRAINZ_NEXT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
const TIMEOUT: Duration = Duration::from_secs(15);
/// Search hits below this MusicBrainz score aren't trusted to be the artist.
const MIN_SCORE: i64 = 90;

/// Wikipedia editions offered in Preferences: (language code, label).
pub const WIKIPEDIA_LANGUAGES: [(&str, &str); 10] = [
    ("en", "English"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("pt", "Português"),
    ("sv", "Svenska"),
    ("ja", "日本語"),
];

/// What the user chose in Preferences.
#[derive(Debug, Clone, PartialEq)]
pub struct LookupOptions {
    /// Wikipedia language code to use, or `None` to skip Wikipedia.
    pub wikipedia_language: Option<String>,
}

impl LookupOptions {
    /// Cache key for `artist` under these options: the same artist looked
    /// up with another Wikipedia setting is a different result.
    pub fn cache_key(&self, artist: &str) -> String {
        format!(
            "{}|{}",
            artist.trim().to_lowercase(),
            self.wikipedia_language.as_deref().unwrap_or("-")
        )
    }

    /// The services a lookup contacts, for the BIO tab's notice.
    pub fn services(&self) -> &'static str {
        if self.wikipedia_language.is_some() {
            "MusicBrainz and Wikipedia"
        } else {
            "MusicBrainz"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistInfo {
    pub name: String,
    /// "Person", "Group", "Orchestra", ...
    pub kind: String,
    /// Country (ISO code as MusicBrainz gives it, e.g. "NL").
    pub country: String,
    pub begin: String,
    pub end: String,
    pub ended: bool,
    /// MusicBrainz's note telling namesakes apart, e.g. "Canadian artist".
    #[serde(default)]
    pub disambiguation: String,
    pub genres: Vec<String>,
    /// Wikipedia's summary of the artist; empty without Wikipedia.
    pub summary: String,
    /// Where "Read more" goes: the Wikipedia article, else MusicBrainz.
    pub link: String,
    /// Whether `summary` came from Wikipedia (for its CC BY-SA credit).
    pub from_wikipedia: bool,
}

impl ArtistInfo {
    /// "Group · NL · 1999–2016", leaving out what isn't known.
    pub fn meta_line(&self) -> String {
        let years = match (self.begin.get(..4), self.end.get(..4)) {
            (Some(begin), Some(end)) => format!("{begin}–{end}"),
            (Some(begin), None) if self.ended => format!("{begin}–?"),
            (Some(begin), None) => format!("{begin}–"),
            _ => String::new(),
        };
        let line = [self.kind.as_str(), self.country.as_str(), years.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        match (line.is_empty(), self.disambiguation.is_empty()) {
            (_, true) => line,
            (true, false) => self.disambiguation.clone(),
            (false, false) => format!("{line} \u{2014} {}", self.disambiguation),
        }
    }
}

/// One of several artists sharing a name, for the user to pick from.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    /// "Group · US — A Cappella group, formerly SONOS".
    pub detail: String,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The info, plus the photo's encoded bytes (JPEG/PNG) if there is one.
    Found(Box<ArtistInfo>, Option<Vec<u8>>),
    /// Several artists have exactly this name; the user picks one (then
    /// [`look_up_id`]).
    Ambiguous(Vec<Candidate>),
    NotFound,
}

/// Looks `artist` up online, using the track's `title` to tell namesakes
/// apart. Blocking (several HTTP requests, ~1-4 s): call it off the UI
/// thread.
pub fn look_up(artist: &str, title: &str, options: &LookupOptions) -> Result<Outcome, String> {
    let candidates = credit_candidates(artist);

    // The recording identifies *which* artist of that name made it.
    if !title.trim().is_empty() {
        let url = format!(
            "https://musicbrainz.org/ws/2/recording/?query={}&limit=5&fmt=json",
            urlencode(&format!(
                "recording:\"{}\" AND artist:\"{}\"",
                title.replace('"', ""),
                candidates[0].replace('"', "")
            ))
        );
        if let Some(id) = artist_from_recordings(&get_json(&url)?, &candidates) {
            return look_up_id(&id, options);
        }
    }

    // Otherwise by name: exact matches only.
    for name in &candidates {
        let url = format!(
            "https://musicbrainz.org/ws/2/artist/?query={}&limit=10&fmt=json",
            urlencode(&format!("artist:\"{}\"", name.replace('"', "")))
        );
        let mut matches = exact_matches(&get_json(&url)?, name);
        match matches.len() {
            0 => continue,
            1 => return look_up_id(&matches.remove(0).id, options),
            _ => return Ok(Outcome::Ambiguous(matches)),
        }
    }
    Ok(Outcome::NotFound)
}

/// Fetches the artist with MusicBrainz id `mbid` (and, if enabled, their
/// Wikipedia biography). Blocking.
pub fn look_up_id(mbid: &str, options: &LookupOptions) -> Result<Outcome, String> {
    let artist_json = get_json(&format!(
        "https://musicbrainz.org/ws/2/artist/{mbid}?inc=url-rels+genres&fmt=json"
    ))?;
    let (mut info, wikidata) = parse_artist(&artist_json, mbid);

    let mut photo = None;
    if let (Some(language), Some(wikidata)) = (&options.wikipedia_language, wikidata) {
        let entity = get_json(&format!(
            "https://www.wikidata.org/wiki/Special:EntityData/{wikidata}.json"
        ))?;
        if let Some((lang, title)) = wikipedia_title(&entity, &wikidata, language) {
            let summary = get_json(&format!(
                "https://{lang}.wikipedia.org/api/rest_v1/page/summary/{}",
                urlencode(&title.replace(' ', "_"))
            ))?;
            let page = parse_summary(&summary);
            if !page.extract.is_empty() {
                info.summary = page.extract;
                info.from_wikipedia = true;
            }
            if let Some(url) = page.page_url {
                info.link = url;
            }
            if let Some(url) = page.thumbnail_url {
                // A missing photo isn't worth failing the lookup over.
                match get_bytes(&url) {
                    Ok(bytes) => photo = Some(bytes),
                    Err(e) => tracing::warn!("couldn't download the artist photo: {e}"),
                }
            }
        }
    }
    Ok(Outcome::Found(Box::new(info), photo))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(USER_AGENT)
        // Statuses are handled in `get` (to retry 503s), not as errors.
        .http_status_as_error(false)
        .build()
        .into()
}

/// Waits until MusicBrainz may be asked again (other hosts don't wait).
fn wait_for_turn(url: &str) {
    if host(url) != "musicbrainz.org" {
        return;
    }
    // Held while sleeping, on purpose: concurrent lookups queue up.
    let mut next = MUSICBRAINZ_NEXT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(at) = *next {
        let now = std::time::Instant::now();
        if at > now {
            std::thread::sleep(at - now);
        }
    }
    *next = Some(std::time::Instant::now() + MUSICBRAINZ_DELAY);
}

/// GETs `url`, retrying when the service says it's busy (503/429) --
/// after its `Retry-After` if it gives one, else 1 s, 2 s, ... -- and
/// turning failures into messages fit for the BIO panel.
fn get(url: &str, accept: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    let service = service_name(url);
    for attempt in 0..ATTEMPTS {
        wait_for_turn(url);
        tracing::debug!("GET {url}");
        let response = agent()
            .get(url)
            .header("Accept", accept)
            .call()
            .map_err(|e| format!("couldn't reach {service}: {e}"))?;
        let status = response.status().as_u16();
        if status == 503 || status == 429 {
            if attempt + 1 == ATTEMPTS {
                break;
            }
            let wait = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or(Duration::from_secs(1 << attempt));
            if wait > MAX_RETRY_AFTER {
                break;
            }
            tracing::debug!("{service} busy (HTTP {status}), retrying in {wait:?}");
            std::thread::sleep(wait);
            continue;
        }
        if !(200..300).contains(&status) {
            return Err(format!("{service} answered with an error (HTTP {status})."));
        }
        return Ok(response);
    }
    Err(format!(
        "{service} is busy right now (too many requests). Try again in a moment."
    ))
}

fn get_json(url: &str) -> Result<Value, String> {
    let body = get(url, "application/json")?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("bad response from {}: {e}", service_name(url)))?;
    serde_json::from_str(&body).map_err(|e| format!("bad response from {}: {e}", service_name(url)))
}

fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    get(url, "image/*")?
        .body_mut()
        .with_config()
        .limit(5 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

/// "MusicBrainz", "Wikipedia", ... for messages.
fn service_name(url: &str) -> &str {
    let host = host(url);
    if host.ends_with("musicbrainz.org") {
        "MusicBrainz"
    } else if host.ends_with("wikidata.org") {
        "Wikidata"
    } else if host.ends_with("wikipedia.org") || host.ends_with("wikimedia.org") {
        "Wikipedia"
    } else {
        host
    }
}

fn host(url: &str) -> &str {
    url.split("://").nth(1).and_then(|rest| rest.split('/').next()).unwrap_or(url)
}

fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Names to search for, most specific first: the whole credit, then
/// without "feat. ..." guests, then the first of several listed artists.
/// "Above & Beyond feat. Zoë Johnston" -> [that, "Above & Beyond",
/// "Above"]; an `&` can be part of a name, so the full forms come first.
fn credit_candidates(credit: &str) -> Vec<String> {
    let cut_at = |text: &str, separators: &[&str]| -> String {
        let lower = text.to_lowercase();
        separators
            .iter()
            .filter_map(|sep| lower.find(sep))
            .min()
            .map_or(text, |at| &text[..at])
            .trim()
            .to_string()
    };
    let full = credit.trim().to_string();
    let without_guests = cut_at(&full, &[" feat. ", " feat ", " featuring ", " ft. ", " ft "]);
    let first_listed = cut_at(&without_guests, &[" & ", ", ", " x ", " vs. ", " vs "]);
    let mut candidates = Vec::new();
    for name in [full, without_guests, first_listed] {
        if !name.is_empty() && !candidates.contains(&name) {
            candidates.push(name);
        }
    }
    candidates
}

/// Artists in a search result whose name (or sort name) is exactly `name`
/// (ignoring case) and that score `MIN_SCORE`+. A merely similar name
/// ("Kush Arora" for "Arora") is a different artist.
fn exact_matches(json: &Value, name: &str) -> Vec<Candidate> {
    let wanted = name.trim().to_lowercase();
    json["artists"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["score"].as_i64().unwrap_or(0) >= MIN_SCORE)
        .filter(|a| {
            a["name"].as_str().is_some_and(|n| n.to_lowercase() == wanted)
                || a["sort-name"].as_str().is_some_and(|n| n.to_lowercase() == wanted)
        })
        .filter_map(|a| {
            let text = |key: &str| a[key].as_str().unwrap_or_default().to_string();
            let kind_country = [text("type"), text("country")]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let detail = match (kind_country.is_empty(), text("disambiguation")) {
                (_, d) if d.is_empty() => kind_country,
                (true, d) => d,
                (false, d) => format!("{kind_country} \u{2014} {d}"),
            };
            Some(Candidate {
                id: a["id"].as_str()?.to_string(),
                name: text("name"),
                detail,
            })
        })
        .collect()
}

/// The artist credited on a matching recording, if one of the credited
/// names is one of `names` -- this pins down *which* artist of that name.
fn artist_from_recordings(json: &Value, names: &[String]) -> Option<String> {
    let wanted: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    json["recordings"]
        .as_array()?
        .iter()
        .filter(|r| r["score"].as_i64().unwrap_or(0) >= MIN_SCORE)
        .flat_map(|r| r["artist-credit"].as_array().into_iter().flatten())
        .find(|credit| {
            credit["artist"]["name"]
                .as_str()
                .is_some_and(|n| wanted.contains(&n.to_lowercase()))
        })
        .and_then(|credit| credit["artist"]["id"].as_str().map(str::to_string))
}

/// Info from a MusicBrainz artist lookup, plus its Wikidata id if linked.
fn parse_artist(json: &Value, mbid: &str) -> (ArtistInfo, Option<String>) {
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let mut genres: Vec<(i64, String)> = json["genres"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| Some((g["count"].as_i64().unwrap_or(0), g["name"].as_str()?.to_string())))
        .collect();
    genres.sort_by_key(|genre| std::cmp::Reverse(genre.0));
    let wikidata = json["relations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|rel| rel["type"].as_str() == Some("wikidata"))
        .filter_map(|rel| rel["url"]["resource"].as_str())
        .find_map(|url| url.rsplit('/').next().filter(|id| id.starts_with('Q')).map(str::to_string));
    let info = ArtistInfo {
        name: text(&json["name"]),
        kind: text(&json["type"]),
        country: text(&json["country"]),
        begin: text(&json["life-span"]["begin"]),
        end: text(&json["life-span"]["end"]),
        ended: json["life-span"]["ended"].as_bool().unwrap_or(false),
        disambiguation: text(&json["disambiguation"]),
        genres: genres.into_iter().take(5).map(|(_, name)| name).collect(),
        summary: String::new(),
        link: format!("https://musicbrainz.org/artist/{mbid}"),
        from_wikipedia: false,
    };
    (info, wikidata)
}

/// The Wikipedia article title for `language`, else English: (language,
/// title).
fn wikipedia_title(entity: &Value, wikidata: &str, language: &str) -> Option<(String, String)> {
    let sitelinks = &entity["entities"][wikidata]["sitelinks"];
    [language, "en"].into_iter().find_map(|lang| {
        sitelinks[format!("{lang}wiki")]["title"]
            .as_str()
            .map(|title| (lang.to_string(), title.to_string()))
    })
}

struct Summary {
    extract: String,
    thumbnail_url: Option<String>,
    page_url: Option<String>,
}

fn parse_summary(json: &Value) -> Summary {
    // A disambiguation page isn't a biography.
    let extract = if json["type"].as_str() == Some("disambiguation") {
        String::new()
    } else {
        json["extract"].as_str().unwrap_or_default().to_string()
    };
    Summary {
        extract,
        // Used exactly as given: Wikimedia only serves its own standard
        // thumbnail widths (a rewritten "320px-" URL gets a 400).
        thumbnail_url: json["thumbnail"]["source"].as_str().map(str::to_string),
        page_url: json["content_urls"]["desktop"]["page"].as_str().map(str::to_string),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn name_search_keeps_only_exact_matches_and_describes_namesakes() {
        // The real MusicBrainz answer for "Arora": four namesakes and a
        // similar-but-different name.
        let results = json!({ "artists": [
            { "id": "g", "name": "ARORA", "sort-name": "ARORA", "type": "Group", "score": 100,
              "disambiguation": "A Cappella group, formerly SONOS" },
            { "id": "f", "name": "Arora", "sort-name": "Arora", "score": 98,
              "disambiguation": "Fictional character from Dead as Disco" },
            { "id": "c", "name": "ARORA", "sort-name": "ARORA", "type": "Person", "country": "CA",
              "score": 98, "disambiguation": "Canadian artist" },
            { "id": "k", "name": "Kush Arora", "sort-name": "Arora, Kush", "type": "Person", "score": 95 },
            { "id": "w", "name": "Arora", "sort-name": "Arora", "score": 40 }
        ]});
        let matches = exact_matches(&results, "Arora");
        let ids: Vec<_> = matches.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["g", "f", "c"]);
        assert_eq!(matches[0].detail, "Group \u{2014} A Cappella group, formerly SONOS");
        assert_eq!(matches[1].detail, "Fictional character from Dead as Disco");
        assert_eq!(matches[2].detail, "Person · CA \u{2014} Canadian artist");
        // Sort names count as exact too.
        let armin = json!({ "artists": [
            { "id": "a", "name": "Armin van Buuren", "sort-name": "Buuren, Armin van", "score": 100 }
        ]});
        assert_eq!(exact_matches(&armin, "buuren, armin van").len(), 1);
    }

    #[test]
    fn a_matching_recording_pins_down_the_artist() {
        let recordings = json!({ "recordings": [
            { "score": 100, "title": "Helpless", "artist-credit": [
                { "name": "Alexander Popov", "artist": { "id": "pop", "name": "Alexander Popov" } },
                { "name": "Myon", "artist": { "id": "myon", "name": "Myon" } }
            ]},
            { "score": 60, "title": "Other", "artist-credit": [
                { "name": "Arora", "artist": { "id": "wrong", "name": "Arora" } }
            ]}
        ]});
        let names = credit_candidates("Alexander Popov & Myon");
        assert_eq!(artist_from_recordings(&recordings, &names).as_deref(), Some("pop"));
        // Low-scoring recordings don't count.
        assert_eq!(artist_from_recordings(&recordings, &["Arora".to_string()]), None);
    }

    #[test]
    fn parses_a_musicbrainz_artist() {
        let artist = json!({
            "name": "Armin van Buuren", "type": "Person", "country": "NL",
            "life-span": { "begin": "1976-12-25", "end": null, "ended": false },
            "genres": [
                { "name": "progressive trance", "count": 3 },
                { "name": "trance", "count": 9 }
            ],
            "relations": [
                { "type": "official homepage", "url": { "resource": "https://www.arminvanbuuren.com/" } },
                { "type": "wikidata", "url": { "resource": "https://www.wikidata.org/wiki/Q190419" } }
            ]
        });
        let (info, wikidata) = parse_artist(&artist, "477b8c0c");
        assert_eq!(wikidata.as_deref(), Some("Q190419"));
        assert_eq!(info.genres, ["trance", "progressive trance"]);
        assert_eq!(info.meta_line(), "Person · NL · 1976–");
        assert_eq!(info.link, "https://musicbrainz.org/artist/477b8c0c");

        let group = ArtistInfo {
            kind: "Group".into(),
            begin: "1999".into(),
            end: "2016-05".into(),
            ended: true,
            ..info
        };
        assert_eq!(group.meta_line(), "Group · NL · 1999–2016");
    }

    #[test]
    fn picks_the_wikipedia_language_with_english_fallback() {
        let entity = json!({ "entities": { "Q1": { "sitelinks": {
            "enwiki": { "title": "Armin van Buuren" },
            "dewiki": { "title": "Armin van Buuren (DJ)" }
        }}}});
        assert_eq!(
            wikipedia_title(&entity, "Q1", "de"),
            Some(("de".into(), "Armin van Buuren (DJ)".into()))
        );
        assert_eq!(
            wikipedia_title(&entity, "Q1", "ja"),
            Some(("en".into(), "Armin van Buuren".into()))
        );
    }

    #[test]
    fn parses_a_wikipedia_summary_and_skips_disambiguation_pages() {
        let page = json!({
            "type": "standard",
            "extract": "Armin van Buuren is a Dutch DJ.",
            "thumbnail": { "source": "https://thumb.wikimedia.org/x/thumb/a/ab/Armin.jpg/330px-Armin.jpg?utm_source=en.wikipedia.org" },
            "content_urls": { "desktop": { "page": "https://en.wikipedia.org/wiki/Armin_van_Buuren" } }
        });
        let summary = parse_summary(&page);
        assert_eq!(summary.extract, "Armin van Buuren is a Dutch DJ.");
        assert_eq!(
            summary.thumbnail_url.as_deref(),
            Some("https://thumb.wikimedia.org/x/thumb/a/ab/Armin.jpg/330px-Armin.jpg?utm_source=en.wikipedia.org")
        );
        assert_eq!(summary.page_url.as_deref(), Some("https://en.wikipedia.org/wiki/Armin_van_Buuren"));
        let ambiguous = json!({ "type": "disambiguation", "extract": "Arora may refer to:" });
        assert_eq!(parse_summary(&ambiguous).extract, "");
    }

    #[test]
    fn joint_credits_try_the_full_names_before_the_first_artist() {
        assert_eq!(
            credit_candidates("Above & Beyond feat. Zoë Johnston"),
            ["Above & Beyond feat. Zoë Johnston", "Above & Beyond", "Above"]
        );
        assert_eq!(credit_candidates("Alexander Popov & Myon"), ["Alexander Popov & Myon", "Alexander Popov"]);
        assert_eq!(credit_candidates("HUNTR/X, EJAE, REI AMI"), ["HUNTR/X, EJAE, REI AMI", "HUNTR/X"]);
        assert_eq!(credit_candidates("Florence and the Machine"), ["Florence and the Machine"]);
        assert_eq!(credit_candidates(" Armin van Buuren "), ["Armin van Buuren"]);
        assert_eq!(urlencode("artist:\"Zoë\""), "artist%3A%22Zo%C3%AB%22");
    }

    #[test]
    fn cache_keys_and_notice_follow_the_options() {
        let with = LookupOptions { wikipedia_language: Some("de".into()) };
        let without = LookupOptions { wikipedia_language: None };
        assert_eq!(with.cache_key(" Armin van Buuren "), "armin van buuren|de");
        assert_ne!(with.cache_key("x"), without.cache_key("x"));
        assert_eq!(without.services(), "MusicBrainz");
    }

    /// Serves `responses` in order on a local port, one per connection.
    fn serve(responses: Vec<&'static str>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let _ = stream.read(&mut request);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        format!("http://{address}/")
    }

    const BUSY: &str = "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const OK: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}";

    #[test]
    fn busy_responses_are_retried() {
        let url = serve(vec![BUSY, BUSY, OK]);
        assert_eq!(get_json(&url).unwrap(), json!({ "ok": true }));
    }

    #[test]
    fn a_service_that_stays_busy_gives_a_readable_error() {
        let url = serve(vec![BUSY, BUSY, BUSY]);
        let error = get_json(&url).unwrap_err();
        assert!(error.contains("is busy right now"), "{error}");
        let not_found = serve(vec!["HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"]);
        assert!(get_json(&not_found).unwrap_err().contains("HTTP 404"));
    }

    #[test]
    fn musicbrainz_requests_are_spaced_out_across_threads() {
        let url = "https://musicbrainz.org/ws/2/artist/";
        let start = std::time::Instant::now();
        let threads: Vec<_> = (0..3).map(|_| std::thread::spawn(move || wait_for_turn(url))).collect();
        for thread in threads {
            thread.join().unwrap();
        }
        // Three turns: the 2nd and 3rd each wait ~1.1 s for the one before.
        assert!(start.elapsed() >= MUSICBRAINZ_DELAY * 2, "{:?}", start.elapsed());
        assert_eq!(service_name("https://upload.wikimedia.org/x.jpg"), "Wikipedia");
    }
}
