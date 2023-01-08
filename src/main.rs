use rand::Rng;
use core::panic;
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    fs, thread,
    time::{self, Duration}, cell::Cell, hash::{Hash, Hasher},
};

use memcache::Client;

const CACHE_KEY: &str = "temporary";
const FILENAME: &str = "large_file_test.pdf";

const SERVERS: &[&str] = &[
    "memcache://172.18.0.2:11211?connect_timeout=2", //memcached1
    "memcache://172.18.0.3:11211?connect_timeout=2", //memcached2
    "memcache://172.18.0.4:11211?connect_timeout=2", //memcached3
];

struct ClientPool {
    servers: HashMap<String, bool>,
    clients: Vec<Client>,
    active_clients: Cell<u32>,
}

impl ClientPool {
    
    fn new () -> ClientPool {
        ClientPool { 
            servers: HashMap::from([]), 
            clients: vec![], 
            active_clients: Cell::new(0) 
        }
    }

    fn hash_function(key: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        return hasher.finish();
    }

    fn add_server(&mut self, url: &str) {
        let server_key = url.to_string().clone();
        let connect_result = memcache::connect(url);
        match connect_result {
            Ok(c) => {
                self.clients.push(c);
                self.servers.insert(server_key, true);
                self.active_clients.set(self.active_clients.get() + 1);
                ()
            },
            Err(_) => {
                self.servers.insert(server_key.clone(), false); 
                ()
            }
        }
    }

    fn get_active_clients(&self) -> u32 {
        self.active_clients.get()
    }

    fn get_index_from_key(&self, key: &str) -> usize {
        let hashed_key = ClientPool::hash_function(key);
        hashed_key as usize % (self.get_active_clients() as usize)
    }

    fn check_active_clients(&self) -> Option<u8>{
        if self.get_active_clients() == 0 {
            println!("No active clients");
            return None;
        }
        Some(0)
    }

    fn get_connection(&self, key: &str) -> Option<&Client> {
        if let None = self.check_active_clients() {
            return None;
        }
        Some(&(self.clients[self.get_index_from_key(key)]))
    }

    fn remove_client_from_key(&mut self, key: &str) {
        let index = self.get_index_from_key(key);
        self.active_clients.set(self.active_clients.get() - 1);
        self.clients.swap_remove(index);
        ()
    }

    fn get(&mut self, key: &str) -> Option<String> {

        let client_opt = self.get_connection(key);
        if let None = client_opt {
            return None;
        }

        let client = client_opt.unwrap();

        let get_res = client.get(key);

        return match get_res {
            Ok(element) => element,
            Err(_) => {
                self.remove_client_from_key(key);
                None
            }
        }
    }

    fn set(&mut self, key: &str, value: String, expiration: u32) {
        let client_opt = self.get_connection(key);
        if let None = client_opt {
            return ();
        }
        let client = client_opt.unwrap();
        let set_result = client.set(key, value, expiration);
        match set_result {
            Ok(_) => (),
            Err(_) => self.remove_client_from_key(key)
        }
    }

    fn get_inactive_servers(&mut self) -> Vec<String> {
        let mut collect = vec![];
        for (server_url, value) in self.servers.iter() {
            if !(*value) {
                collect.push(server_url.clone());
            }
        }
        collect
    }

    fn reconnect_unactive_servers (&mut self) {
        for server in self.get_inactive_servers() {
            self.add_server(server.as_str());
        }
    }

}

fn main() {
    
    test2();
    
}

fn test2 () {
    
    print!("Starting setting up servers. . .");
    let mut pool = ClientPool::new();
    for server in SERVERS {
        pool.add_server(server);
    }

    let mut iterations = 1;

    loop {
        println!("*** Starting iteration n° {}", iterations);
        let now = time::Instant::now();
        println!("Active servers: {}", pool.get_active_clients());
        
        let mut value = pool.get(CACHE_KEY);
        if let None = value {
            let tmp = execute_long_query();
            pool.set(CACHE_KEY, tmp.clone(), 600);
            value = Some(tmp);
        }
    
        println!(
            "I got {} in {} seconds", if let Some(x) = value { x } else { String::from("Nothing") }, now.elapsed().as_secs()
        );

        println!("Trying to reconnect to inactive servers");
        pool.reconnect_unactive_servers();
        println!("Let me rest 5 seconds before next iteration");
        thread::sleep(Duration::from_secs(5));
        iterations += 1;
    }

}

#[allow(dead_code)]
fn test1 () {
    // List of clients that are actually responding
    let mut clients: Vec<Client> = vec![];
    let now = time::Instant::now();

    for server in SERVERS {
        let connect_attempt = memcache::connect(*server);

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

    println!(
        "{} servers on {} are connected",
        clients.len(),
        SERVERS.len()
    );
    let mut value: Option<String> = None;
    if clients.len() > 0 {
        let hashed_key = (clients[0].hash_function)(CACHE_KEY);
        let index = (hashed_key as usize) % clients.len();
        let trial = clients[index].get(CACHE_KEY);
        value = match trial {
            Ok(cached) => cached,
            Err(_) => None,
        };
    }

    if let None = value {
        let new_val = execute_long_query();
        let hashed_key = (clients[0].hash_function)(CACHE_KEY);
        let index = (hashed_key as usize) % clients.len();
        let opt_res = clients[index].set(CACHE_KEY, &new_val, 600); // 5 minutes
        match opt_res {
            Ok(_) => (),
            Err(_) => (),
        }
        value = Some(new_val);
    }

    println!(
        "I got {} in {} seconds", if let Some(x) = value { x } else { String::from("Nothing") }, now.elapsed().as_secs()
    );
}

fn execute_long_query() -> String {
    thread::sleep(Duration::from_secs(10));
    String::from("Here's your value")
}

#[allow(dead_code)]
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

#[allow(dead_code)]
fn print_stats(client: &Client) {
    let stats = client.stats().unwrap();
    for (first, second) in stats.iter() {
        println!("Server id {}", first);
        println!("Curr_items: {}", second.get("curr_items").unwrap());
    }
}
