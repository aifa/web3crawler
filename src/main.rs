extern crate spider;
extern crate env_logger;
extern crate serde_json;

use spider::website::Website;
use std::io::{self, Write};
use ipfs_api::{IpfsApi, IpfsClient};
use std::fs::File;
use std::path::Path;
use actix_web::{post, App, HttpServer, Responder, web};
use tokio::sync::mpsc;
use threadpool::ThreadPool;
use std::io::Cursor;

extern crate leveldb_minimal;

use leveldb_minimal::database::Database;
use leveldb_minimal::kv::KV;
use leveldb_minimal::options::{Options,WriteOptions,ReadOptions};

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

struct AppState{
      tx: tokio::sync::mpsc::Sender<String>,
}

#[actix_web::main] // or #[tokio::main]
async fn main() -> std::io::Result<()> {
    let (tx, mut rx) = mpsc::channel(32);
    let app_state = web::Data::new( AppState{
        tx: tx.clone(),
    });

    tokio::spawn(async move {
        let pool = ThreadPool::new(10);
        while let Some(message) = rx.recv().await {
            println!("GOT = {}", message);
               pool.execute(move || {
                   let ipns_link=scrape(message);
                   println!("Done with:{:?}", ipns_link );
               });
         };
        pool.join();
    });

    HttpServer::new( move || {
        App::new()
        .app_data(app_state.clone())
        .service(crawl_and_scrape)
    })
    .bind(("127.0.0.1", 6008))?
    .run()
    .await
}

#[post("/crawl_and_scrape")]
async fn crawl_and_scrape(body: String, data: web::Data<AppState>) -> impl Responder {
   //let ok =scrape(body).await;
   let input=body.clone();
   let tx = data.tx.clone();
   tokio::spawn(async move {
       match tx.send(body).await{
           Ok(body) => eprintln!("sent for scraping: {:?}", body),
           Err(e) => eprintln!("error accessing the scraping queue: {}", e),
       };
   });

  format!("Received for processing {}:", input)
}

fn write_to_file(temp_file: String, content: &[u8]) -> std::io::Result<()> {
    let mut file = File::create(temp_file)?;
    file.write_all(content)?;

    io::stdout().write_all(content).unwrap();

    Ok(())
}

async fn  content_to_ipfs(content : &'static str) -> String {
    tracing_subscriber::fmt::init();
    eprintln!("connecting to localhost:5001...");
    let client = IpfsClient::default();
    let mut file_hash=String::from("");
    let data = Cursor::new(content);

    match client.version().await {
        Ok(version) => eprintln!("version: {:?}", version.version),
        Err(e) => eprintln!("error getting version: {}", e),
    }

    match client.add(data).await {
        Ok(res) => {
            eprintln!("added json file: {:?}", res.hash);
            file_hash=res.hash;
        }
        Err(e) => eprintln!("error adding json file: {}", e),
    };

    return file_hash.to_string();
}

async fn  file_to_ipfs(file_name : &'static str) -> String {
    tracing_subscriber::fmt::init();
    eprintln!("connecting to localhost:5001...");
    let client = IpfsClient::default();


    match client.version().await {
        Ok(version) => eprintln!("version: {:?}", version.version),
        Err(e) => eprintln!("error getting version: {}", e),
    }

    let file = File::open(file_name).expect("could not read source file");

    let mut file_hash=String::from("");
    let mut publish_name=String::from("");

    match client.add(file).await {
        Ok(res) => {
            eprintln!("added json file: {:?}", res.hash);
            file_hash=res.hash;
        }
        Err(e) => eprintln!("error adding json file: {}", e),
    };

    match client.name_publish(&file_hash, true, None, None, None).await {
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

fn scrape(url: String) -> io::Result<String>{
    eprintln!("got request for:{:?}", url);
    //scrape(body)
    use serde_json::{json};
    let mut website: Website = Website::new(&url);

    let temp_file: &'static str = "./tempfile";

    configure(&mut website);

    website.scrape();
/*
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

    let link_database = match Database::open(link_path, url_options) {
        Ok(db) => { db },
        Err(e) => { panic!("failed to open database: {:?}", e) }
    };

    let mut hasher = DefaultHasher::new();

    for page in website.get_pages() {
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
        hasher.write(page.get_url().as_bytes());
        let url_hash = hasher.finish();
        match content_database.put(write_opts, url_hash.to_string().as_bytes(), &serde_json::to_string(&page_json).unwrap().as_bytes()) {
            Ok(_) => { () },
            Err(e) => { panic!("failed to write to content database: {:?}", e) }
        };

        match link_database.put(write_opts, url_hash.to_string().as_bytes(), page.get_url().as_bytes()) {
            Ok(_) => { () },
            Err(e) => { panic!("failed to write to url database: {:?}", e) }
        };
    }
*/
    Ok(format!("{} was succefully scraped", url))
}

fn configure(website: &mut Website){
    website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
    website.configuration.respect_robots_txt = true;
    website.configuration.subdomains = true;
    website.configuration.delay = 250; // Defaults to 250 ms
    website.configuration.concurrency = 10; // Defaults to number of cpus available * 4
    website.configuration.user_agent = "myapp/version".to_string(); // Defaults to spider/x.y.z, where x.y.z is the library version
    //website.on_link_find_callback = |s| { println!("link target: {}", s); s }; // Callback to run on each link find
}
