use core::fmt;
use std::{
    collections::{hash_map::DefaultHasher, BTreeMap, HashMap},
    fs,
    hash::{Hash, Hasher},
    io::Cursor,
    thread,
    time::{self, Duration},
};

use std::error::Error;

use anyhow::Result as AnyHowResult;
use async_raft::{
    async_trait::async_trait,
    raft::{
        AppendEntriesRequest, AppendEntriesResponse, Entry, EntryPayload, InstallSnapshotRequest,
        InstallSnapshotResponse, MembershipConfig, VoteRequest, VoteResponse,
    },
    storage::{CurrentSnapshotData, HardState, InitialState},
    AppData, AppDataResponse, NodeId, Raft, RaftNetwork, RaftStorage,
};
use memcache::{Client, MemcacheError};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::sync::{RwLockReadGuard, RwLockWriteGuard};

//use thiserror::Error;

//const CACHE_KEY: &str = "temporary";
//const FILENAME: &str = "large_file_test.pdf";

const SERVERS: &[&str] = &[
    "memcache://172.18.0.2:11211?connect_timeout=2", //memcached1
    "memcache://172.18.0.3:11211?connect_timeout=2", //memcached2
    "memcache://172.18.0.4:11211?connect_timeout=2", //memcached3
];

#[derive(Debug, Clone)]
struct ShutdownError;

// Generation of an error is completely separate from how it is displayed.
// There's no need to be concerned about cluttering complex logic with the display style.
//
// Note that we don't store any extra info about the errors. This means we can't state
// which string failed to parse without modifying our types to carry that information.
impl fmt::Display for ShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Error. Shutting down")
    }
}

impl Error for ShutdownError {}

//impl StdError

// This is the application data request used by Memrafted.
// It contains the minimum fields necessary to store information
// in the memcached server
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ClientRequest {
    key: String,
    value: String,
    expiration: u32,
}

impl AppData for ClientRequest {}

// This struct represents the response, which for the current moment
// is only a String.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientResponse(Option<String>);

impl AppDataResponse for ClientResponse {}

/// A type which emulates a network transport and implements the `RaftNetwork` trait.
/*pub struct RaftRouter {
    // ... some internal state ...
}

#[async_trait]
impl RaftNetwork<ClientRequest> for RaftRouter {
    // Send an AppendEntries RPC to the target Raft node (§5).
    async fn append_entries(&self, target: u64, rpc: AppendEntriesRequest<ClientRequest>) -> AnyHowResult<AppendEntriesResponse> {
        // ... snip ...
    }

    // Send an InstallSnapshot RPC to the target Raft node (§7).
    async fn install_snapshot(&self, target: u64, rpc: InstallSnapshotRequest) -> AnyHowResult<InstallSnapshotResponse> {
        // ... snip ...
    }

    // Send a RequestVote RPC to the target Raft node (§5).
    async fn vote(&self, target: u64, rpc: VoteRequest) -> AnyHowResult<VoteResponse> {
        //  ... snip ...
    }
}*/

pub struct MemStore {
    id: NodeId,
    // Server url
    server_url: String,
    // Client for communicaiton with Memcached server
    client: RwLock<Option<Client>>,
    // Set of logs. Each command is a mutation of the state, mapped to a term.
    // RwLock is an asynchronous reader-writer lock, at most one writer at any point in time
    log: RwLock<BTreeMap<u64, Entry<ClientRequest>>>,

    last_applied_log: RwLock<u64>,
}

impl MemStore {
    pub fn new(id: NodeId, url: String) -> Self {
        let client_res = memcache::connect(url.clone());
        let client = match client_res {
            Ok(c) => Some(c),
            Err(_) => None,
        };
        Self {
            id,
            server_url: url,
            client: RwLock::new(client),
            log: RwLock::new(BTreeMap::new()),
            last_applied_log: RwLock::new(0)
        }
    }
}

#[async_trait]
impl RaftStorage<ClientRequest, ClientResponse> for MemStore {
    type Snapshot = Cursor<Vec<u8>>;
    type ShutdownError = ShutdownError;

    async fn get_membership_config(&self) -> AnyHowResult<MembershipConfig> {
        let log = self.log.read().await;
        let cfg_opt = log.values().rev().find_map(|entry| match &entry.payload {
            EntryPayload::ConfigChange(cfg) => Some(cfg.membership.clone()),
            EntryPayload::SnapshotPointer(snap) => Some(snap.membership.clone()),
            _ => None,
        });
        Ok(match cfg_opt {
            Some(cfg) => cfg,
            None => MembershipConfig::new_initial(self.id),
        })
    }

    async fn get_initial_state(&self) -> AnyHowResult<InitialState> {
        let new = InitialState::new_initial(self.id);
        Ok(new)
    }

    async fn save_hard_state(&self, hs: &HardState) -> AnyHowResult<()> {
        //*self.hs.write().await = Some(hs.clone());
        Ok(())
    }

    async fn get_log_entries(&self, start: u64, stop: u64) -> AnyHowResult<Vec<Entry<ClientRequest>>> {
        // Invalid request, return empty vec.
        if start > stop {
            //tracing::error!("invalid request, start > stop");
            return Ok(vec![]);
        }
        let log = self.log.read().await;
        Ok(log.range(start..stop).map(|(_, val)| val.clone()).collect())
    }

    async fn delete_logs_from(&self, start: u64, stop: Option<u64>) -> AnyHowResult<()> {
        if stop.as_ref().map(|stop| &start > stop).unwrap_or(false) {
            //tracing::error!("invalid request, start > stop");
            return Ok(());
        }
        let mut log = self.log.write().await;

        // If a stop point was specified, delete from start until the given stop point.
        if let Some(stop) = stop.as_ref() {
            for key in start..*stop {
                log.remove(&key);
            }
            return Ok(());
        }
        // Else, just split off the remainder.
        log.split_off(&start);
        Ok(())
    }

    async fn append_entry_to_log(&self, entry: &Entry<ClientRequest>) -> AnyHowResult<()> {
        let mut log = self.log.write().await;
        log.insert(entry.index, entry.clone());
        Ok(())
    }

    async fn replicate_to_log(&self, entries: &[Entry<ClientRequest>]) -> AnyHowResult<()> {
        let mut log = self.log.write().await;
        for entry in entries {
            log.insert(entry.index, entry.clone());
        }
        Ok(())
    }    

    async fn apply_entry_to_state_machine(&self, index: &u64, data: &ClientRequest) -> AnyHowResult<ClientResponse> {
        // To make the call to Memcached
        
        Ok(ClientResponse(None))
    }

    async fn replicate_to_state_machine(&self, entries: &[(&u64, &ClientRequest)]) -> AnyHowResult<()> {
        // let mut sm = self.sm.write().await;
        // for (index, data) in entries {
        //     sm.last_applied_log = **index;
        //     if let Some((serial, _)) = sm.client_serial_responses.get(&data.client) {
        //         if serial == &data.serial {
        //             continue;
        //         }
        //     }
        //     let previous = sm.client_status.insert(data.client.clone(), data.status.clone());
        //     sm.client_serial_responses.insert(data.client.clone(), (data.serial, previous.clone()));
        // }
        Ok(())
    }

    async fn do_log_compaction(&self) -> AnyHowResult<CurrentSnapshotData<Self::Snapshot>> {
        todo!()
    }

    async fn create_snapshot(&self) -> AnyHowResult<(String, Box<Self::Snapshot>)> {
        todo!()
    }

    async fn finalize_snapshot_installation(
        &self, index: u64, term: u64, delete_through: Option<u64>, id: String, snapshot: Box<Self::Snapshot>,
    ) -> AnyHowResult<()> {
        
        Ok(())
    }

    async fn get_current_snapshot(&self) -> AnyHowResult<Option<CurrentSnapshotData<Self::Snapshot>>> {
        todo!()
    }



}

//pub type MemRaft = Raft<ClientRequest, ClientResponse, RaftRouter, MemStore>;

fn main() {}

/*impl Memrafted {

    pub fn is_active(&self) -> bool {
        match self.client {
            Some(_) => true,
            None => false
        }
    }

    pub  fn get(&self, key: &str) -> Option<String> {
        None
    }

    pub  fn set(&self, key: &str, value: &str, expiration: u32)  {
        //self.client.set(key, value, expiration)
    }
}


struct ClientPool {
    servers: HashMap<String, bool>,
    clients: Vec<MemraftedClient>,
}




impl ClientPool {

    fn new () -> ClientPool {
        ClientPool {
            servers: HashMap::from([]),
            clients: vec![],
        }
    }

    fn hash_function(key: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        return hasher.finish();
    }

    fn add_server(&mut self, url: &str) {
        let connect_result = memcache::connect(url);
        match connect_result {
            Ok(c) => {
                self.clients.push(
                    MemraftedClient {
                        server_url: String::from(url),
                        client: c
                    }
                );
                self.servers.insert(String::from(url), true);
                ()
            },
            Err(_) => {
                self.servers.insert(String::from(url), false);
                ()
            }
        }
    }

    fn get_active_clients(&self) -> usize {
        self.clients.len()
    }

    fn get_index_from_key(&self, key: &str) -> usize {
        let hashed_key = ClientPool::hash_function(key);
        let res = hashed_key as usize % self.get_active_clients();
        println!("Key: '{}', index: {}", key, res);
        res
    }

    fn check_active_clients(&self) -> bool {
        if self.get_active_clients() == 0 {
            println!("No active clients");
            return false;
        }
        true
    }

    fn get_connection(&self, key: &str) -> Option<&MemraftedClient> {
        if !self.check_active_clients() {
            return None;
        }
        Some(&(self.clients[self.get_index_from_key(key)]))
    }

    fn remove_client_from_key(&mut self, key: &str) {
        let index = self.get_index_from_key(key);
        let server_url = self.get_connection(key).unwrap().server_url.clone();
        self.clients.swap_remove(index);
        self.servers.insert(server_url, false);
        ()
    }

    fn get(&mut self, key: &str) -> Option<String> {

        let client_opt = self.get_connection(key);
        if let None = client_opt {
            return None;
        }

        let my_client = client_opt.unwrap();

        let get_res = my_client.get(key);

        return match get_res {
            Ok(element) => element,
            Err(_) => {
                self.remove_client_from_key(key);
                // Retry get
                self.get(key)
            }
        }
    }

    fn set(&mut self, key: &str, value: &str, expiration: u32) {
        let client_opt = self.get_connection(key);
        if let None = client_opt {
            return ();
        }
        let client = client_opt.unwrap();
        let set_result = client.set(key, value, expiration);
        match set_result {
            Ok(_) => (),
            Err(_) => {
                self.remove_client_from_key(key);
                self.set(key, value, expiration)
            }
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

    fn print_current_state(&self) {
        println!("Printing current state");
        for (key, value) in self.servers.iter() {
            println!("Server '{}': {}", key, value);
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
            pool.set(CACHE_KEY, &tmp, 600);
            value = Some(tmp);
        }

        println!(
            "I got {} in {} seconds", if let Some(x) = value { x } else { String::from("Nothing") }, now.elapsed().as_secs()
        );

        pool.print_current_state();
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
*/
