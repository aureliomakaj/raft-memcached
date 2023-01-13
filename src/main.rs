use std::{thread::sleep, time::Duration, collections::HashMap};

use actix_web::{get, web, App, HttpServer, Responder, Result};
use serde::Serialize;


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

fn mock_storage_status() -> StorageStatus {
    let disk_total = 137438953472.0; // 50 gb
    let disk_used = 50465865728.0; // 47 gb
    let disk_free = disk_total - disk_used;

    StorageStatus {
        disk_total,
        disk_used,
        disk_free,
        critical: disk_free < (disk_total * 0.9),
    }
}

struct Repository {}

// Struct that simulates a repository that interacts with a database 
impl Repository {
    fn get_users() -> Vec<User> {
        mock_user_list()
    }

    fn get_storage_status () -> StorageStatus {
        mock_storage_status()
    }
}

struct CacheValue {
    value: String,
    expiration: u64
}

// Local cache, not scalable
struct Memrafted {
    map: HashMap<String, CacheValue>
}

impl Memrafted {
    fn get(key: &str) -> Option<String> {
        None
    }
}

#[get("/users")]
async fn users() -> Result<impl Responder> {
    let users = Repository::get_users();
    Ok(web::Json(users))
}

#[get("/storage-status")]
async fn storage_status() -> Result<impl Responder> {
    sleep(Duration::from_secs(30));
    let storage = Repository::get_storage_status();
    Ok(web::Json(storage))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| 
                    App::new()
                        .service(users)
                        .service(storage_status)
                )
                .bind(("127.0.0.1", 8080))?
                .run()
                .await
}
