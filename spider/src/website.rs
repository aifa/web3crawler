use crate::black_list::contains;
use crate::configuration::Configuration;
use crate::page::Page;
use crate::utils::{log};
use reqwest::blocking::{Client};
use rayon::ThreadPool;
use rayon::ThreadPoolBuilder;
use robotparser_fork::RobotFileParser;
use hashbrown::HashSet;
use std::{time::{Duration}};
use std::sync::mpsc::{channel, Sender, Receiver};
use reqwest::header::CONNECTION;
use reqwest::header;
use tokio::time::sleep;

extern crate leveldb_minimal;
extern crate serde_json;

use leveldb_minimal::database::Database;
use leveldb_minimal::kv::KV;
use leveldb_minimal::options::{Options,WriteOptions,ReadOptions};
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::path::Path;
use serde_json::{json};
/// Represents a website to crawl and gather all links.
/// ```rust
/// use spider::website::Website;
/// let mut website = Website::new("http://example.com");
/// website.crawl();
/// // `Website` will be filled with `Pages` when crawled. To get them, just use
/// for page in website.get_pages() {
///     // do something
/// }
/// ```
pub struct Website<'a> {
    /// configuration properties for website.
    pub configuration: Configuration,
    /// this is a start URL given when instanciate with `new`.
    pub domain: String,
    /// contains all non-visited URL.
    links: HashSet<String>,
    /// contains all visited URL.
    links_visited: HashSet<String>,
    /// contains page visited
    pages: Vec<Page>,
    /// callback when a link is found.
    pub on_link_find_callback: fn(String) -> String,
    /// Robot.txt parser holder.
    robot_file_parser: RobotFileParser<'a>,
    //level DB database for URLs
    url_database: Database,
    // level DB database for html content
    content_database: Database,
}

type Message = HashSet<String>;

impl<'a> Website<'a> {

    /// Initialize Website object with a start link to crawl.
    pub fn new(domain: &str) -> Self {
        let content_path = Path::new("./webpages");
        let link_path = Path::new("./links");

        let mut content_options = Options::new();
        content_options.create_if_missing = true;

        let mut url_options = Options::new();
        url_options.create_if_missing = true;

        let storage_setup = || -> (Database, Database) {
            let content_path = Path::new("./webpages");
            let link_path = Path::new("./links");

            let mut content_options = Options::new();
            content_options.create_if_missing = true;

            let mut url_options = Options::new();
            url_options.create_if_missing = true;
            let content_database = match Database::open(content_path, content_options) {
                Ok(db) => { db },
                Err(e) => { panic!("failed to open database: {:?}", e) }
            };
            let url_database = match Database::open(link_path, url_options) {
                Ok(db) => { db },
                Err(e) => { panic!("failed to open database: {:?}", e) }
            };
            (url_database, content_database)
        };
        let (url_store, content_store) = storage_setup();

        Self {
            configuration: Configuration::new(),
            links_visited: HashSet::new(),
            pages: Vec::new(),
            robot_file_parser: RobotFileParser::new(&format!("{}/robots.txt", domain)), // TODO: lazy establish
            links: HashSet::from([format!("{}/", domain)]),
            on_link_find_callback: |s| s,
            domain: domain.to_owned(),
            content_database: content_store,
            url_database: url_store

        }
    }

    /// page getter
    pub fn get_pages(&self) -> Vec<Page> {
        if !self.pages.is_empty(){
            self.pages.clone()
        } else {
            self.links_visited.iter().map(|l| Page::build(l, "")).collect()
        }
    }

    /// links visited getter
    pub fn get_links(&self) -> &HashSet<String> {
        &self.links_visited
    }

    /// crawl delay getter
    fn get_delay(&self) -> Duration {
        Duration::from_millis(self.configuration.delay)
    }

    /// configure the robots parser on initial crawl attempt and run
    pub fn configure_robots_parser(&mut self) {
        if self.configuration.respect_robots_txt && self.robot_file_parser.mtime() == 0 {
            self.robot_file_parser.user_agent = self.configuration.user_agent.to_string();
            self.robot_file_parser.read();
            self.configuration.delay = self
                .robot_file_parser
                .get_crawl_delay(&self.robot_file_parser.user_agent) // returns the crawl delay in seconds
                .unwrap_or(self.get_delay())
                .as_millis() as u64;
        }
    }

    /// configure http client
    fn configure_http_client(&mut self, user_agent: Option<String>) -> Client {
        let mut headers = header::HeaderMap::new();
        headers.insert(CONNECTION, header::HeaderValue::from_static("keep-alive"));

        Client::builder()
            .default_headers(headers)
            .user_agent(user_agent.unwrap_or(self.configuration.user_agent.to_string()))
            .build()
            .expect("Failed building client.")
    }

    /// configure rayon thread pool
    fn create_thread_pool(&mut self) -> ThreadPool {
        ThreadPoolBuilder::new()
            .num_threads(self.configuration.concurrency)
            .build()
            .expect("Failed building thread pool.")
    }

    /// setup config for crawl
    fn setup(&mut self) -> Client {
        self.configure_robots_parser();
        let client = self.configure_http_client(None);
        client
    }

    /// Start to crawl website with async parallelization
    pub fn crawl(&mut self) {
        let client = self.setup();

        self.crawl_concurrent(&client);
    }

    /// Start to scrape website with async parallelization
    pub fn scrape(&mut self) {
        let client = self.setup();

        self.scrape_concurrent(&client);
    }

    /// Start to crawl website in sync
    pub fn crawl_sync(&mut self) {
        let client = self.setup();

        self.crawl_sequential(&client);
    }

    /// Start to crawl website concurrently
    fn crawl_concurrent(&mut self, client: &Client) {
        let pool = self.create_thread_pool();
        let delay = self.configuration.delay;
        let subdomains = self.configuration.subdomains;
        let tld = self.configuration.tld;
        let delay_enabled = delay > 0;
        let on_link_find_callback = self.on_link_find_callback;

        // crawl while links exists
        while !self.links.is_empty() {
            let (tx, rx): (Sender<Message>, Receiver<Message>) = channel();

            for link in self.links.iter() {
                if !self.is_allowed(link) {
                    continue;
                }
                log("fetch", link);

                self.links_visited.insert(link.into());

                let link = link.clone();
                let tx = tx.clone();
                let cx = client.clone();

                pool.spawn(move || {
                    if delay_enabled {
                        tokio_sleep(&Duration::from_millis(delay));
                    }
                    let link_result = on_link_find_callback(link);
                    let page = Page::new(&link_result, &cx);
                    let links = page.links(subdomains, tld);

                    tx.send(links).unwrap();
                });
            }

            drop(tx);

            let mut new_links: HashSet<String> = HashSet::new();

            rx.into_iter().for_each(|links| {
                new_links.extend(links);
            });

            self.links = &new_links - &self.links_visited;
        }
    }

    /// Start to crawl website sequential
    fn crawl_sequential(&mut self, client: &Client) {
        let delay = self.configuration.delay;
        let subdomains = self.configuration.subdomains;
        let tld = self.configuration.tld;
        let delay_enabled = delay > 0;
        let on_link_find_callback = self.on_link_find_callback;

        // crawl while links exists
        while !self.links.is_empty() {
            let mut new_links: HashSet<String> = HashSet::new();

            for link in self.links.iter() {
                if !self.is_allowed(link) {
                    continue;
                }
                log("fetch", link);
                self.links_visited.insert(link.into());
                if delay_enabled {
                    tokio_sleep(&Duration::from_millis(delay));
                }

                let link = link.clone();
                let cx = client.clone();
                let link_result = on_link_find_callback(link);
                let page = Page::new(&link_result, &cx);
                let links = page.links(subdomains, tld);

                new_links.extend(links);
            }

            self.links = &new_links - &self.links_visited;
        }
    }

    /// Start to scape website concurrently and store html
    fn scrape_concurrent(&mut self, client: &Client) {
        let pool = self.create_thread_pool();
        let delay = self.configuration.delay;
        let delay_enabled = delay > 0;
        let on_link_find_callback = self.on_link_find_callback;

        // crawl while links exists
        while !self.links.is_empty() {
            let (tx, rx): (Sender<Page>, Receiver<Page>) = channel();

            for link in self.links.iter() {
                if !self.is_allowed(link) {
                    continue;
                }
                log("fetch", link);

                self.links_visited.insert(link.into());

                let link = link.clone();
                let tx = tx.clone();
                let cx = client.clone();

                pool.spawn(move || {
                    if delay_enabled {
                        tokio_sleep(&Duration::from_millis(delay));
                    }
                    let link_result = on_link_find_callback(link);
                    let page = Page::new(&link_result, &cx);

                    tx.send(page).unwrap();
                });
            }

            drop(tx);

            let mut new_links: HashSet<String> = HashSet::new();

            rx.into_iter().for_each(|page| {
                let links = page.links(self.configuration.subdomains, self.configuration.tld);
                new_links.extend(links);
                self.store_page(&page);
                //self.pages.push(page);
            });

            self.links = &new_links - &self.links_visited;
        }
    }
    pub fn store_page(&self, page: &Page){
        let mut links: Vec<String> = vec![];
        let page_links = page.links(false, false);
        links.extend(page_links);

        let page_json = json!({
            "url": page.get_url(),
            "links": links,
            "html": page.get_html(),
        });
        //let ipfs_link=content_to_ipfs(&serde_json::to_string(&page_json).unwrap()).await;

        let write_opts = WriteOptions::new();
        let mut hash_factory = DefaultHasher::new();
        hash_factory.write(page.get_url().as_bytes());
        let url_hash = hash_factory.finish();
        match self.content_database.put(write_opts, url_hash.to_string().as_bytes(), &serde_json::to_string(&page_json).unwrap().as_bytes()) {
            Ok(_) => { () },
            Err(e) => { panic!("failed to write to content database: {:?}", e) }
        };

        match self.url_database.put(write_opts, url_hash.to_string().as_bytes(), page.get_url().as_bytes()) {
            Ok(_) => { () },
            Err(e) => { panic!("failed to write to url database: {:?}", e) }
        };
    }
    /// return `true` if URL:
    ///
    /// - is not already crawled
    /// - is not blacklisted
    /// - is not forbidden in robot.txt file (if parameter is defined)
    pub fn is_allowed(&self, link: &String) -> bool {
        if self.links_visited.contains(link) {
            return false;
        }
        if contains(&self.configuration.blacklist_url, link) {
            return false;
        }
        if self.configuration.respect_robots_txt && !self.is_allowed_robots(link) {
            return false;
        }

        true
    }


    /// return `true` if URL:
    ///
    /// - is not forbidden in robot.txt file (if parameter is defined)
    pub fn is_allowed_robots(&self, link: &String) -> bool {
        self.robot_file_parser.can_fetch("*", link)
    }
}

impl<'a> Drop for Website<'a> {
    fn drop(&mut self) {}
}

// blocking sleep keeping thread alive
#[tokio::main]
async fn tokio_sleep(delay: &Duration){
    sleep(*delay).await;
}

#[test]
fn crawl() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.crawl();
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[test]
fn scrape() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.scrape();
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );

    assert_eq!(
        website.get_pages()[0].get_html().is_empty(),
        false
    );
}

#[test]
fn crawl_subsequential() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.delay = 250;
    website.crawl_sync();
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[test]
fn crawl_invalid() {
    let url = "https://w.com";
    let mut website: Website = Website::new(url);
    website.crawl();
    let mut uniq = HashSet::new();
    uniq.insert(format!("{}/", url.to_string())); // TODO: remove trailing slash mutate

    assert_eq!(website.links_visited, uniq); // only the target url should exist
}

#[test]
fn crawl_link_callback() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.on_link_find_callback = |s| {
       log("callback link target: {}", &s);
        s
    };
    website.crawl();
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[test]
fn not_crawl_blacklist() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website
        .configuration
        .blacklist_url
        .push("https://choosealicense.com/licenses/".to_string());
    website.crawl();
    assert!(
        !website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[test]
#[cfg(feature = "regex")]
fn not_crawl_blacklist_regex() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website
        .configuration
        .blacklist_url
        .push("/choosealicense.com/".to_string());
    website.crawl();
    assert_eq!(website.links_visited.len(), 0);
}

#[test]
#[cfg(feature = "ua_generator")]
fn randomize_website_agent() {
    let website: Website = Website::new("https://choosealicense.com");

    assert_eq!(website.configuration.user_agent.is_empty(), false);
}

#[test]
fn test_respect_robots_txt() {
    let mut website: Website = Website::new("https://stackoverflow.com");
    website.configuration.respect_robots_txt = true;
    assert_eq!(website.configuration.delay, 250);
    assert!(!website.is_allowed(&"https://stackoverflow.com/posts/".to_string()));

    // test match for bing bot
    let mut website_second: Website = Website::new("https://www.mongodb.com");
    website_second.configuration.respect_robots_txt = true;
    website_second.configuration.user_agent = "bingbot".into();
    website_second.configure_robots_parser();
    assert_eq!(
        website_second.configuration.user_agent,
        website_second.robot_file_parser.user_agent
    );
    assert_eq!(website_second.configuration.delay, 60000); // should equal one minute in ms

    // test crawl delay with wildcard agent [DOES not work when using set agent]
    let mut website_third: Website = Website::new("https://www.mongodb.com");
    website_third.configuration.respect_robots_txt = true;
    website_third.configure_robots_parser();

    assert_eq!(website_third.configuration.delay, 10000); // should equal 10 seconds in ms
}

#[test]
fn test_crawl_subdomains() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.subdomains = true;
    website.crawl();
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}

#[test]
fn test_crawl_tld() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.subdomains = true;
    website.crawl();
    assert!(
        website
            .links_visited
            .contains(&"https://choosealicense.com/licenses/".to_string()),
        "{:?}",
        website.links_visited
    );
}


#[test]
fn test_link_duplicates() {
    fn has_unique_elements<T>(iter: T) -> bool
    where
        T: IntoIterator,
        T::Item: Eq + std::hash::Hash,
    {
        let mut uniq = HashSet::new();
        iter.into_iter().all(move |x| uniq.insert(x))
    }

    let mut website: Website = Website::new("http://0.0.0.0:8000");
    website.crawl();

    assert!(has_unique_elements(&website.links_visited));
}
