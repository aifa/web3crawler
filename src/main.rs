extern crate spider;
extern crate env_logger;
extern crate serde_json;

use spider::website::Website;
use std::io::{self, Write};
use ipfs_api::{IpfsApi, IpfsClient};
use std::io::Cursor;
use std::fs::File;
use std::io::prelude::*;

fn main() -> io::Result<()>{
    use serde_json::{json};
    let mut website: Website = Website::new("https://choosealicense.com");

    let temp_file: &'static str = "./tempfile";

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

    let mut file = File::create(temp_file)?;
    file.write_all(j.as_bytes())?;

    io::stdout().write_all(j.as_bytes()).unwrap();
        //write json outpot to ipfs
    let future = writeToIpfs(temp_file);

    Ok(())
}

#[tokio::main]
async fn writeToIpfs(file_name : &'static str) -> String {
    tracing_subscriber::fmt::init();
    eprintln!("connecting to localhost:5001...");
    let client = IpfsClient::default();


    match client.version().await {
        Ok(version) => eprintln!("version: {:?}", version.version),
        Err(e) => eprintln!("error getting version: {}", e),
    }

    let file = File::open(file_name).expect("could not read source file");

    let mut file_hash=String::from("");;
    let mut publish_name=String::from("");

    match client.add(file).await {
        Ok(res) => {
            eprintln!("added json file: {:?}", res.hash);
            file_hash=res.hash;
        }
        Err(e) => eprintln!("error adding json file: {}", e),
    };

    let publish = match client.name_publish(&file_hash, true, None, None, None).await {
        Ok(publish) => {
            eprintln!("published {} to: /ipns/{}", file_hash, &publish.name);
            publish_name=publish.name;
        }
        Err(e) => {
            eprintln!("error publishing name: {}", e);
        }
    };

    return publish_name.to_string();
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
