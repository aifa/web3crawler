extern crate spider;
extern crate env_logger;
extern crate serde_json;

use spider::website::Website;
use std::io::{self, Write};
use marine_rs_sdk::marine;
use marine_rs_sdk::module_manifest;

fn main() {
}

#[marine]
pub fn scrape(domain: String) -> Vev<u8> {

    use serde_json::{json};
    let mut website: Website = Website::new(domain);

    configure(&mut website);

    website.scrape();

    let mut page_objects: Vec<_> = vec![];

    for page in website.get_pages() {
        let mut links: Vec<String> = vec![];
        let page_links = page.links(false, false);
        links.extend(page_links);

        let page_json = json!({
            "url": page.get_url(),
            "links": links,
            "html": page.get_html(),
        });

        page_objects.push(page_json);
    }

    let j = serde_json::to_string_pretty(&page_objects).unwrap();

    io::stdout().write_all(j.as_bytes()).unwrap();

    return j.as_bytes();
}

fn configure(website: &mut Website){
    website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
    website.configuration.respect_robots_txt = true;
    website.configuration.subdomains = true;
    website.configuration.delay = 2000; // Defaults to 250 ms
    website.configuration.concurrency = 10; // Defaults to number of cpus available * 4
    website.configuration.user_agent = "myapp/version".to_string(); // Defaults to spider/x.y.z, where x.y.z is the library version
    website.on_link_find_callback = |s| { println!("link target: {}", s); s }; // Callback to run on each link find
}
