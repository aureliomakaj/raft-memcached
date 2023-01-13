use std::{thread::sleep, time::{Duration, Instant}, collections::HashMap, sync::Mutex};

use actix_web::{get, web, App, HttpServer, Responder, Result};
use serde::Serialize;

const STORAGE_USAGE_KEY: &str = "storage_usage_key";

#[derive(Serialize)]
struct User {
    name: String,
    email: String,
    phone: String,
}

#[derive(Serialize)]
struct StorageStatus {
    disk_total: f64,
    disk_used: f64,
    disk_free: f64,
    critical: bool,
}

fn mock_user_list() -> Vec<User> {
    let mut res = vec![];
    res.push(User {
        name: String::from("Kristen Cannon"),
        email: String::from("mauris.ut@google.com"),
        phone: String::from("(612) 562-3195"),
    });
    res.push(User {
        name: String::from("Akeem Larsen"),
        email: String::from("morbi.metus@outlook.net"),
        phone: String::from("1-234-781-8772"),
    });
    res.push(User {
        name: String::from("Illana Bowen"),
        email: String::from("rutrum.lorem.ac@yahoo.net"),
        phone: String::from("(438) 666-7031"),
    });

    res
}

struct Repository {}

// Struct that simulates a repository that interacts with a database 
impl Repository {
    fn get_users() -> Vec<User> {
        mock_user_list()
    }

    fn get_disk_total () -> f64 {
        137438953472.0
    }

    fn get_disk_used() -> f64 {
        sleep(Duration::from_secs(30));
        50465865728.0
    }
}
#[derive(Clone)]
enum CacheValueType {
    String(String),
    F64(f64),
}

#[derive(Clone)]
struct CacheValue {
    // The value stored. For simplicity we will use only strings
    value: CacheValueType,
    // Expiration time in seconds. How much the data should be available 
    expiration: u64,

    // Time of when the value was inserted or updated
    creation: Instant,
}

// Local cache, not scalable
struct Memrafted {
    map:  HashMap<String, CacheValue>
}

impl Memrafted {
    pub fn new() -> Self {
        Self {
            map: HashMap::new()
        }
    }

    pub fn get(&mut self, key: &str) -> Option<CacheValueType> {
        // Get the value from the hashmap
        let in_cache = self.map.get(key);
        match in_cache {
            Some(value) => {
                // The value was present.
                // Check if the expiration hasn't been reached
                if Memrafted::is_expired(value) {
                    // Expiration reached. Remove the key from the hashmap
                    self.map.remove(key);
                    // Return None as the value wasn't valid anymore
                    None
                }else {
                    Some(value.value.clone())
                }
            },
            None => None
        }
    }

    pub fn set(&mut self, key: &str, value: CacheValueType, expiration: u64) {
        self.map.insert(String::from(key), CacheValue { 
            value, 
            expiration,
            creation: Instant::now()
        });
        ()
    }

    fn is_expired(value: &CacheValue) -> bool {
        let elapsed = value.creation.elapsed().as_secs();
        elapsed > value.expiration
    }
}

impl Clone for Memrafted {
    fn clone(&self) -> Self {
        Self { map: self.map.clone() }
    }
}

#[get("/users")]
async fn users() -> Result<impl Responder> {
    println!("Got request for /users");
    // Here we know that the query isn't long, so we don't use the cache
    let users = Repository::get_users();
    Ok(web::Json(users))
}

#[get("/storage-status")]
async fn storage_status(app_state: web::Data<AppState>) -> Result<impl Responder> {
    println!("Got request for /storage-status");

    let mut cache = app_state.memrafted.lock().unwrap();
    // This query is fast
    let disk_total = Repository::get_disk_total();
    // This query is slow. Let's check if we have saved it before in the cache
    let disk_usage_cache = (*cache).get(STORAGE_USAGE_KEY);
    let mut disk_used = 0.0;
    if let None = disk_usage_cache {
        // Nothing in cache. Make long query
        disk_used = Repository::get_disk_used();
        // Save for future needs
        (*cache).set(STORAGE_USAGE_KEY, CacheValueType::F64(disk_used), 360);
    }

    let disk_free = disk_total - disk_used;

    let storage = StorageStatus {
        disk_total,
        disk_used,
        disk_free,
        critical: disk_free < (disk_total * 0.9),
    };
    // = Repository::get_storage_status();
    Ok(web::Json(storage))
}

struct AppState {
    memrafted: Mutex<Memrafted>
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    println!("Starting server...");
    println!("Server ready. Listening on 127.0.0.1:8080");

    let appstate = web::Data::new( AppState {
        memrafted: Mutex::new(Memrafted::new())
    });

    HttpServer::new(move || 
                    App::new()
                        .app_data(appstate.clone())
                        .service(users)
                        .service(storage_status)
                )
                .bind(("127.0.0.1", 8080))?
                .run()
                .await
}
