//! Pages you've been to, for suggestions in the address bar. Each visit is
//! a line in `history.jsonl`; a page visited again is a newer line for the
//! same address, and reading keeps the newest of each.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use protocol::Visit;

/// Kept when there are more lines than this many pages times two.
const KEEP: usize = 5000;

pub struct History {
    path: PathBuf,
    pages: Mutex<HashMap<String, Page>>,
}

#[derive(Clone)]
struct Page {
    visit: Visit,
    /// How many times it's been visited.
    times: u32,
}

impl History {
    pub fn load(path: PathBuf) -> Self {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut pages: HashMap<String, Page> = HashMap::new();
        let mut lines = 0;
        for visit in text
            .lines()
            .filter_map(|line| serde_json::from_str::<Visit>(line).ok())
        {
            lines += 1;
            pages
                .entry(visit.url.clone())
                .and_modify(|page| {
                    // A title arriving for the same visit isn't another visit.
                    if visit.at.saturating_sub(page.visit.at) >= 60_000 {
                        page.times += 1;
                    }
                    page.visit = visit.clone();
                })
                .or_insert(Page { visit, times: 1 });
        }
        let history = History {
            path,
            pages: Mutex::new(pages),
        };
        if lines > KEEP * 2 {
            history.compact();
        }
        history
    }

    /// A page loaded, or its title changed. Pages of the browser's own
    /// (blank pages, settings, devtools) aren't kept.
    pub fn record(&self, url: &str, title: &str, at: u64) {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return;
        }
        let mut pages = self.pages.lock().expect("not poisoned");
        let visit = Visit {
            url: url.to_string(),
            title: title.to_string(),
            at,
        };
        match pages.get_mut(url) {
            // The same visit, now with its title: not another visit.
            Some(page) if at.saturating_sub(page.visit.at) < 60_000 => {
                if page.visit.title == title || title.is_empty() {
                    return;
                }
                page.visit.title = title.to_string();
            }
            Some(page) => {
                page.times += 1;
                page.visit = visit.clone();
            }
            None => {
                pages.insert(
                    url.to_string(),
                    Page {
                        visit: visit.clone(),
                        times: 1,
                    },
                );
            }
        }
        let line = serde_json::to_string(&pages[url].visit).expect("visits serialize");
        drop(pages);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| writeln!(file, "{line}"));
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<Visit> {
        let pages = self.pages.lock().expect("not poisoned");
        rank(pages.values().cloned().collect(), query, limit)
    }

    /// Rewrites the file with one line for each page, the newest pages kept.
    fn compact(&self) {
        let pages = self.pages.lock().expect("not poisoned");
        let mut newest: Vec<&Page> = pages.values().collect();
        newest.sort_by_key(|page| std::cmp::Reverse(page.visit.at));
        let text: String = newest
            .into_iter()
            .take(KEEP)
            .map(|page| serde_json::to_string(&page.visit).expect("visits serialize") + "\n")
            .collect();
        let _ = crate::config::write_file(&self.path, &text);
    }
}

/// Pages whose address or title holds every word typed, the best first:
/// where the address starts with what's typed, then the often and lately
/// visited. With nothing typed, the latest.
fn rank(pages: Vec<Page>, query: &str, limit: usize) -> Vec<Visit> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let newest = pages.iter().map(|page| page.visit.at).max().unwrap_or(0);
    let mut found: Vec<(f64, Visit)> = pages
        .into_iter()
        .filter_map(|page| {
            let url = page.visit.url.to_lowercase();
            let bare = url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("www.");
            let title = page.visit.title.to_lowercase();
            if !words.iter().all(|w| bare.contains(w) || title.contains(w)) {
                return None;
            }
            let days_old = newest.saturating_sub(page.visit.at) as f64 / 86_400_000.0;
            let mut score = f64::from(page.times).ln_1p() * 2.0 - days_old.min(60.0) / 10.0;
            if !query.is_empty() && bare.starts_with(&query) {
                score += 10.0;
            }
            Some((score, page.visit))
        })
        .collect();
    if query.is_empty() {
        found.sort_by_key(|(_, visit)| std::cmp::Reverse(visit.at));
    } else {
        found.sort_by(|a, b| b.0.total_cmp(&a.0));
    }
    found
        .into_iter()
        .take(limit)
        .map(|(_, visit)| visit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400_000;

    #[test]
    fn keeps_the_newest_of_each_page_and_counts_visits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        let history = History::load(path.clone());
        history.record("https://grafana.shop.internal/d/checkout", "", DAY);
        history.record(
            "https://grafana.shop.internal/d/checkout",
            "Checkout",
            DAY + 500,
        );
        history.record(
            "https://grafana.shop.internal/d/checkout",
            "Checkout",
            3 * DAY,
        );
        history.record("about:blank", "", 3 * DAY);
        let again = History::load(path);
        let found = again.search("", 10);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Checkout");
        assert_eq!(found[0].at, 3 * DAY);
        assert_eq!(
            again.pages.lock().unwrap()["https://grafana.shop.internal/d/checkout"].times,
            2
        );
    }

    #[test]
    fn ranks_what_starts_with_the_query_then_the_often_visited() {
        let page = |url: &str, title: &str, at: u64, times: u32| Page {
            visit: Visit {
                url: url.into(),
                title: title.into(),
                at,
            },
            times,
        };
        let pages = vec![
            page(
                "https://docs.gradle.org/userguide",
                "Gradle docs",
                10 * DAY,
                1,
            ),
            page(
                "https://grafana.shop.internal/d/payments",
                "Payments",
                9 * DAY,
                2,
            ),
            page(
                "https://grafana.shop.internal/d/checkout",
                "Checkout",
                9 * DAY,
                9,
            ),
            page("https://dev.azure.com/shop", "Azure DevOps", 10 * DAY, 30),
        ];
        let urls = |found: Vec<Visit>| found.into_iter().map(|v| v.url).collect::<Vec<_>>();
        assert_eq!(
            urls(rank(pages.clone(), "graf", 5)),
            [
                "https://grafana.shop.internal/d/checkout",
                "https://grafana.shop.internal/d/payments",
            ]
        );
        assert_eq!(
            urls(rank(pages.clone(), "gra docs", 5)),
            ["https://docs.gradle.org/userguide"]
        );
        assert_eq!(rank(pages.clone(), "", 2).len(), 2);
        assert_eq!(
            urls(rank(pages, "", 1)),
            ["https://docs.gradle.org/userguide"]
        );
    }
}
