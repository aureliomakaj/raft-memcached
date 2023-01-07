use rand::Rng;
use std::{collections::HashMap, fs, thread, time};

use memcache::Client;

const CACHE_KEY: &str = "temporary";
const FILENAME: &str = "large_file_test.pdf";

fn main() {
    let servers = [
        "memcache://172.17.0.2:11211?connect_timeout=2",
        "memcache://172.17.0.3:11211?connect_timeout=2",
        "memcache://172.17.0.4:11211?connect_timeout=2",
    ];
    let mut clients: Vec<Client> = vec![];

    let now = time::Instant::now();

    for server in servers {
        let res = memcache::connect(server);
        let client = match res {
            Ok(client) => Some(client),
            Err(_) => {
                println!("Server '{}' not reachable", server);
                None
            }
        };

        match client {
            Some(c) => clients.push(c),
            None => (),
        }
    }

    println!("Number of clients: {}", clients.len());
    let mut ok = false;
    let mut i = 0;
    let mut in_cache: String = String::from("");

    while !ok && i < clients.len() {
        let value_try: Option<String> = clients[i].get(CACHE_KEY).unwrap();
        print_stats(&clients[i]);
        match value_try {
            Some(cached_value) => {
                in_cache = Some(cached_value);
                ok = true;
            }
            None => {
                i += 1;
            }
        }
        println!("Finished attempt: {}; Guard status: {}", i, ok);
    }

    if let None = in_cache {
        let tmp = String::from("Hello");

        let mut rng = rand::thread_rng();
        let server_index = rng.gen_range(0..clients.len());

        if !ok {
            clients[server_index].set(CACHE_KEY, tmp, 3600).unwrap();
        }
    }

    println!("I got {} in {} seconds", value, now.elapsed().as_secs());
}

fn execute_long_query() -> String {
    
}

/*fn read_file() -> HashMap<u8, u32> {
    let contents = fs::read(FILENAME).expect("Should have been able to read the file");

    let mut map: HashMap<u8, u32> = HashMap::from([]);

    for elem in contents.iter() {
        map.insert(
            *elem,
            1 + if map.contains_key(elem) { map[elem] } else { 0 },
        );
    }

    map
}*/

fn print_stats(client: &Client) {
    let stats = client.stats().unwrap();
    for (first, second) in stats.iter() {
        println!("Server id {}", first);
        println!("Curr_items: {}", second.get("curr_items").unwrap());
    }
}
