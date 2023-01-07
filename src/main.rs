use rand::Rng;
use std::{
    collections::HashMap,
    fs, thread,
    time::{self, Duration},
};

use memcache::Client;

const CACHE_KEY: &str = "temporary";
const FILENAME: &str = "large_file_test.pdf";

fn main() {
    let servers = [
        "memcache://172.18.0.2:11211?connect_timeout=2", //memcached1
        "memcache://172.18.0.3:11211?connect_timeout=2", //memcached2
        "memcache://172.18.0.4:11211?connect_timeout=2", //memcached3
    ];


    // List of clients that are actually responding
    let mut clients: Vec<Client> = vec![];

    let now = time::Instant::now();

    for server in servers {
        let connect_attempt = memcache::connect(server);

        let client = match connect_attempt {
            Ok(client) => Some(client),
            Err(_) => {
                println!("Server '{}' not reachable", server);
                None
            }
        };

        // Add client only if connected
        match client {
            Some(c) => clients.push(c),
            None => (),
        }
    }

    println!("{} servers on {} are connected", clients.len(), servers.len());
    let mut value: Option<String> = None;
    if clients.len() > 0 {
        let hashed_key = (clients[0].hash_function)(CACHE_KEY);
        let index = (hashed_key as usize )% clients.len();
        let trial = clients[index].get(CACHE_KEY);
        value = match trial {
            Ok(cached) => cached,
            Err(_) => None
        };
    }

    if let None = value {
        let new_val = execute_long_query();
        let hashed_key = (clients[0].hash_function)(CACHE_KEY);
        let index = (hashed_key as usize )% clients.len();
        let opt_res = clients[index].set(CACHE_KEY, &new_val, 600); // 5 minutes
        match opt_res {
            Ok(_) => (),
            Err(_) => () 
        }
        value = Some(new_val);
    }

    println!("I got {} in {} seconds", if let Some(x) = value {x} else { String::from("Nothing")}, now.elapsed().as_secs());
}

fn execute_long_query() -> String {
    thread::sleep(Duration::from_secs(30));
    String::from("Here's your value")
}

fn read_file() -> HashMap<u8, u32> {
    let contents = fs::read(FILENAME).expect("Should have been able to read the file");

    let mut map: HashMap<u8, u32> = HashMap::from([]);

    for elem in contents.iter() {
        map.insert(
            *elem,
            1 + if map.contains_key(elem) { map[elem] } else { 0 },
        );
    }

    map
}

fn print_stats(client: &Client) {
    let stats = client.stats().unwrap();
    for (first, second) in stats.iter() {
        println!("Server id {}", first);
        println!("Curr_items: {}", second.get("curr_items").unwrap());
    }
}
