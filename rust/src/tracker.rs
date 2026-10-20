use crate::{
    config::Limits,
    model::{Record, Resource, Version},
    storage::Storage,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
};

#[derive(Debug)]
pub enum Error {
    Busy,
    Closed,
    Limit,
    Storage(io::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "tracker queue is full",
            Self::Closed => "tracker is stopped",
            Self::Limit => "subscription or connection limit reached",
            Self::Storage(_) => "resource storage unavailable",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ClientId(u64);
pub struct Update {
    pub text: Arc<str>,
    _budget: OwnedSemaphorePermit,
}
pub struct Connection {
    pub id: ClientId,
    pub updates: mpsc::Receiver<Update>,
    pub cancelled: watch::Receiver<bool>,
    tracker: Tracker,
}
impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.tracker.tx.try_send(Command::Disconnect(self.id));
    }
}
struct Client {
    versions: HashMap<Resource, Version>,
    updates: mpsc::Sender<Update>,
    cancelled: watch::Sender<bool>,
    budget: Arc<Semaphore>,
}
impl Client {
    fn enqueue(&self, text: Arc<str>) -> bool {
        let Ok(budget) = self
            .budget
            .clone()
            .try_acquire_many_owned(text.len() as u32)
        else {
            return false;
        };
        self.updates
            .try_send(Update {
                text,
                _budget: budget,
            })
            .is_ok()
    }
}
type Reply<T> = oneshot::Sender<Result<T, Error>>;
enum Command {
    Connect(Client, Reply<ClientId>),
    Subscribe(ClientId, Resource, Version, Reply<()>),
    Unsubscribe(ClientId, Resource, Reply<()>),
    Publish(Resource, Record, Reply<()>),
    Disconnect(ClientId),
    Shutdown(oneshot::Sender<()>),
    #[cfg(test)]
    Inspect(oneshot::Sender<(usize, usize, usize)>),
}
#[derive(Clone)]
pub struct Tracker {
    tx: mpsc::Sender<Command>,
    limits: Limits,
}
impl Tracker {
    pub fn start(storage: Storage, cache_items: usize, limits: Limits) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(1024);
        let actor = Actor {
            storage,
            cache: Cache::new(cache_items),
            clients: HashMap::new(),
            by_resource: HashMap::new(),
            limits: limits.clone(),
            next_id: 0,
        };
        (Self { tx, limits }, tokio::spawn(actor.run(rx)))
    }
    async fn request<T>(
        &self,
        command: Command,
        reply: oneshot::Receiver<Result<T, Error>>,
    ) -> Result<T, Error> {
        self.tx.try_send(command).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => Error::Busy,
            _ => Error::Closed,
        })?;
        reply.await.unwrap_or(Err(Error::Closed))
    }
    pub async fn connect(&self) -> Result<Connection, Error> {
        let (updates, rx) = mpsc::channel(self.limits.queued_updates);
        let (cancelled, cancel_rx) = watch::channel(false);
        let (reply, response) = oneshot::channel();
        let client = Client {
            versions: HashMap::new(),
            updates,
            cancelled,
            budget: Arc::new(Semaphore::new(
                self.limits.queued_bytes_per_connection as usize,
            )),
        };
        let id = self
            .request(Command::Connect(client, reply), response)
            .await?;
        Ok(Connection {
            id,
            updates: rx,
            cancelled: cancel_rx,
            tracker: self.clone(),
        })
    }
    pub async fn subscribe(
        &self,
        id: ClientId,
        resource: Resource,
        version: Version,
    ) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.request(Command::Subscribe(id, resource, version, tx), rx)
            .await
    }
    pub async fn unsubscribe(&self, id: ClientId, resource: Resource) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.request(Command::Unsubscribe(id, resource, tx), rx)
            .await
    }
    pub async fn publish(&self, resource: Resource, record: Record) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.request(Command::Publish(resource, record, tx), rx)
            .await
    }
    pub async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Command::Shutdown(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }
}
/// Bounded FIFO cache; updating a resource never increases cardinality.
struct Cache {
    records: HashMap<Resource, Record>,
    order: VecDeque<Resource>,
    capacity: usize,
}
impl Cache {
    fn new(capacity: usize) -> Self {
        Self {
            records: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }
    fn invalidate(&mut self, resource: &Resource) {
        self.records.remove(resource);
        self.order.retain(|r| r != resource);
    }
    fn insert(&mut self, resource: Resource, record: Record) {
        if self.capacity == 0 {
            return;
        }
        if !self.records.contains_key(&resource) {
            if self.records.len() == self.capacity
                && let Some(old) = self.order.pop_front()
            {
                self.records.remove(&old);
            }
            self.order.push_back(resource.clone());
        }
        self.records.insert(resource, record);
    }
}
struct Actor {
    storage: Storage,
    cache: Cache,
    clients: HashMap<ClientId, Client>,
    by_resource: HashMap<Resource, HashSet<ClientId>>,
    limits: Limits,
    next_id: u64,
}
impl Actor {
    fn disconnect(&mut self, id: ClientId) {
        if let Some(client) = self.clients.remove(&id) {
            let _ = client.cancelled.send(true);
            for resource in client.versions.keys() {
                self.remove_index(id, resource);
            }
        }
    }
    fn remove_index(&mut self, id: ClientId, resource: &Resource) {
        if let Some(clients) = self.by_resource.get_mut(resource) {
            clients.remove(&id);
            if clients.is_empty() {
                self.by_resource.remove(resource);
            }
        }
    }
    fn prune_disconnected(&mut self) {
        let ids = self
            .clients
            .iter()
            .filter_map(|(id, c)| c.updates.is_closed().then_some(*id))
            .collect::<Vec<_>>();
        for id in ids {
            self.disconnect(id);
        }
    }
    async fn subscribe(
        &mut self,
        id: ClientId,
        resource: Resource,
        version: Version,
    ) -> Result<(), Error> {
        let Some(client) = self.clients.get(&id) else {
            return Err(Error::Closed);
        };
        if client.versions.contains_key(&resource) {
            return Ok(());
        }
        if client.versions.len() >= self.limits.subscriptions_per_connection {
            self.disconnect(id);
            return Err(Error::Limit);
        }
        let record = if let Some(record) = self.cache.records.get(&resource) {
            Some(record.clone())
        } else {
            let record = self.storage.read(&resource).await.map_err(Error::Storage)?;
            if let Some(record) = &record {
                self.cache.insert(resource.clone(), record.clone());
            }
            record
        };
        let client = self.clients.get_mut(&id).ok_or(Error::Closed)?;
        let version = if let Some(record) = record.filter(|r| version.is_older_than(&r.version)) {
            if !client.enqueue(record.message(&resource)) {
                self.disconnect(id);
                return Err(Error::Limit);
            }
            record.version
        } else {
            version
        };
        client.versions.insert(resource.clone(), version);
        self.by_resource.entry(resource).or_default().insert(id);
        Ok(())
    }
    async fn publish(&mut self, resource: Resource, record: Record) -> Result<(), Error> {
        if let Err(error) = self.storage.write(&resource, &record).await {
            // A failed directory sync can follow a successful atomic replacement.
            // Force catch-up from disk instead of serving a possibly stale cache.
            self.cache.invalidate(&resource);
            let affected = self
                .by_resource
                .get(&resource)
                .into_iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            for id in affected {
                self.disconnect(id);
            }
            return Err(Error::Storage(error));
        }
        self.cache.insert(resource.clone(), record.clone());
        let text = record.message(&resource);
        let mut slow = Vec::new();
        for id in self.by_resource.get(&resource).into_iter().flatten() {
            let Some(client) = self.clients.get_mut(id) else {
                continue;
            };
            if client
                .versions
                .get(&resource)
                .is_some_and(|v| v.is_older_than(&record.version))
            {
                if !client.enqueue(text.clone()) {
                    slow.push(*id);
                } else {
                    client
                        .versions
                        .insert(resource.clone(), record.version.clone());
                }
            }
        }
        for id in slow {
            self.disconnect(id);
        }
        Ok(())
    }
    async fn run(mut self, mut rx: mpsc::Receiver<Command>) {
        let mut cleanup = tokio::time::interval(Duration::from_secs(1));
        loop {
            let command = tokio::select! {
                command=rx.recv()=>match command{Some(c)=>c,None=>break},
                _=cleanup.tick()=>{self.prune_disconnected();continue;}
            };
            match command {
                Command::Connect(client, reply) => {
                    self.prune_disconnected();
                    if self.clients.len() >= self.limits.connections {
                        let _ = reply.send(Err(Error::Limit));
                        continue;
                    }
                    let Some(next_id) = self.next_id.checked_add(1) else {
                        let _ = reply.send(Err(Error::Limit));
                        continue;
                    };
                    self.next_id = next_id;
                    let id = ClientId(next_id);
                    self.clients.insert(id, client);
                    if reply.send(Ok(id)).is_err() {
                        self.disconnect(id);
                    }
                }
                Command::Subscribe(id, r, v, reply) => {
                    let result = self.subscribe(id, r, v).await;
                    let _ = reply.send(result);
                }
                Command::Unsubscribe(id, r, reply) => {
                    if let Some(c) = self.clients.get_mut(&id) {
                        c.versions.remove(&r);
                    }
                    self.remove_index(id, &r);
                    let _ = reply.send(Ok(()));
                }
                Command::Publish(r, record, reply) => {
                    let result = self.publish(r, record).await;
                    let _ = reply.send(result);
                }
                Command::Disconnect(id) => self.disconnect(id),
                Command::Shutdown(reply) => {
                    for id in self.clients.keys().copied().collect::<Vec<_>>() {
                        self.disconnect(id);
                    }
                    rx.close();
                    let _ = reply.send(());
                    break;
                }
                #[cfg(test)]
                Command::Inspect(reply) => {
                    let _ = reply.send((
                        self.clients.len(),
                        self.by_resource.len(),
                        self.cache.records.len(),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn resource(name: &str) -> Resource {
        Resource::try_from(format!("/{name}")).unwrap()
    }
    fn record(version: usize) -> Record {
        Record {
            version: Version(version.to_string()),
            data: json!({}),
        }
    }
    async fn counts(tracker: &Tracker) -> (usize, usize, usize) {
        let (tx, rx) = oneshot::channel();
        tracker.tx.send(Command::Inspect(tx)).await.unwrap();
        rx.await.unwrap()
    }
    #[tokio::test]
    async fn cache_counts_resources_and_unsubscribe_reclaims_indexes() {
        let (t, task) = Tracker::start(Storage::open(None).unwrap(), 2, Limits::default());
        let c = t.connect().await.unwrap();
        for n in 0..100 {
            t.publish(resource("same"), record(n)).await.unwrap();
        }
        t.publish(resource("other"), record(1)).await.unwrap();
        assert_eq!(counts(&t).await, (1, 0, 2));
        for n in 0..100 {
            let r = resource(&n.to_string());
            t.subscribe(c.id, r.clone(), Version("0".into()))
                .await
                .unwrap();
            t.unsubscribe(c.id, r).await.unwrap();
        }
        assert_eq!(counts(&t).await, (1, 0, 2));
        drop(c);
        t.shutdown().await;
        task.await.unwrap();
    }
    #[tokio::test]
    async fn slow_client_is_disconnected_and_can_catch_up_again() {
        let limits = Limits {
            queued_updates: 1,
            ..Limits::default()
        };
        let (t, task) = Tracker::start(Storage::open(None).unwrap(), 2, limits);
        let mut c = t.connect().await.unwrap();
        let r = resource("updates");
        t.subscribe(c.id, r.clone(), Version("0".into()))
            .await
            .unwrap();
        t.publish(r.clone(), record(1)).await.unwrap();
        t.publish(r.clone(), record(2)).await.unwrap();
        c.cancelled.changed().await.unwrap();
        assert!(*c.cancelled.borrow());
        assert_eq!(counts(&t).await, (0, 0, 1));
        let mut fresh = t.connect().await.unwrap();
        t.subscribe(fresh.id, r, Version("0".into())).await.unwrap();
        assert!(
            fresh
                .updates
                .recv()
                .await
                .unwrap()
                .text
                .contains("\"version\":\"2\"")
        );
        t.shutdown().await;
        task.await.unwrap();
    }
    #[tokio::test]
    async fn failed_persistence_does_not_publish_or_poison_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(Some(dir.path().to_owned())).unwrap();
        let (t, task) = Tracker::start(storage, 2, Limits::default());
        let mut c = t.connect().await.unwrap();
        let r = resource("updates");
        t.subscribe(c.id, r.clone(), Version("0".into()))
            .await
            .unwrap();
        let moved = dir.path().with_extension("moved");
        std::fs::rename(dir.path(), &moved).unwrap();
        assert!(matches!(
            t.publish(r.clone(), record(1)).await,
            Err(Error::Storage(_))
        ));
        assert_eq!(counts(&t).await, (0, 0, 0));
        assert!(*c.cancelled.borrow());
        assert!(c.updates.try_recv().is_err());
        std::fs::rename(&moved, dir.path()).unwrap();
        let mut c = t.connect().await.unwrap();
        t.subscribe(c.id, r.clone(), Version("0".into()))
            .await
            .unwrap();
        t.publish(r, record(2)).await.unwrap();
        assert!(
            c.updates
                .recv()
                .await
                .unwrap()
                .text
                .contains("\"version\":\"2\"")
        );
        t.shutdown().await;
        task.await.unwrap();
    }
    #[test]
    fn invalidation_preserves_fifo_eviction_after_reinsertion() {
        let mut cache = Cache::new(2);
        cache.insert(resource("a"), record(1));
        cache.insert(resource("b"), record(1));
        cache.invalidate(&resource("a"));
        cache.insert(resource("a"), record(2));
        cache.insert(resource("c"), record(1));
        assert_eq!(cache.records.len(), 2);
        assert_eq!(cache.order.len(), 2);
        assert!(cache.records.contains_key(&resource("a")));
        assert!(cache.records.contains_key(&resource("c")));
        assert!(!cache.records.contains_key(&resource("b")));
    }
    #[tokio::test]
    async fn failure_after_rename_forces_subscribers_to_reload_committed_record() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(Some(dir.path().to_owned())).unwrap();
        let fault = storage.clone();
        let (t, task) = Tracker::start(storage, 2, Limits::default());
        let r = resource("updates");
        t.publish(r.clone(), record(1)).await.unwrap();
        let c = t.connect().await.unwrap();
        t.subscribe(c.id, r.clone(), Version("1".into()))
            .await
            .unwrap();
        let unaffected = t.connect().await.unwrap();
        t.subscribe(unaffected.id, resource("other"), Version("0".into()))
            .await
            .unwrap();
        fault.fail_next_write_after_rename();
        assert!(matches!(
            t.publish(r.clone(), record(2)).await,
            Err(Error::Storage(_))
        ));
        assert!(*c.cancelled.borrow());
        assert!(!*unaffected.cancelled.borrow());
        assert_eq!(counts(&t).await, (1, 1, 0));
        assert_eq!(fault.read(&r).await.unwrap().unwrap().version.0, "2");
        let mut fresh = t.connect().await.unwrap();
        t.subscribe(fresh.id, r.clone(), Version("1".into()))
            .await
            .unwrap();
        assert!(
            fresh
                .updates
                .recv()
                .await
                .unwrap()
                .text
                .contains("\"version\":\"2\"")
        );
        t.publish(r, record(3)).await.unwrap();
        assert!(
            fresh
                .updates
                .recv()
                .await
                .unwrap()
                .text
                .contains("\"version\":\"3\"")
        );
        t.shutdown().await;
        task.await.unwrap();
    }
    #[tokio::test]
    async fn shutdown_drains_accepted_writes_even_when_callers_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(Some(dir.path().to_owned())).unwrap();
        let (t, task) = Tracker::start(storage, 2, Limits::default());
        let r = resource("ordered");
        for n in 1..=20 {
            let (reply, response) = oneshot::channel();
            drop(response);
            t.tx.send(Command::Publish(r.clone(), record(n), reply))
                .await
                .unwrap();
        }
        t.shutdown().await;
        task.await.unwrap();
        let bytes = std::fs::read(Storage::filename(dir.path(), &r)).unwrap();
        assert_eq!(
            serde_json::from_slice::<Record>(&bytes).unwrap().version.0,
            "20"
        );
    }
}
