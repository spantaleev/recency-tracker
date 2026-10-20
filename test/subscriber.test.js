const assert = require('node:assert/strict');
const { once } = require('node:events');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');
const { WebSocket } = require('ws');
const { createServer } = require('../lib/server/subscriber.js');
const createPersister = require('../lib/persister.js');
const createTracker = require('../lib/tracker.js');

const staticPath = path.join(__dirname, '../resources/static');
const clientSource = require('node:fs').readFileSync(path.join(staticPath, 'recency-tracker.js'), 'utf8');
const deadline = () => ({ signal: AbortSignal.timeout(3000) });

async function waitFor(predicate) {
	const expires = Date.now() + 3000;
	while (!predicate()) {
		assert.ok(Date.now() < expires, 'Timed out waiting for expected behavior');
		await new Promise(resolve => setTimeout(resolve, 5));
	}
}

async function startServer(t, server) {
	const sockets = new Set();
	server.on('connection', socket => {
		sockets.add(socket);
		socket.on('close', () => sockets.delete(socket));
	});
	t.after(async () => {
		for (const socket of sockets) socket.destroy();
		if (server.listening) await new Promise(resolve => server.close(resolve));
	});
	server.listen(0, '127.0.0.1');
	await once(server, 'listening', deadline());
	return `http://127.0.0.1:${server.address().port}`;
}

async function createSubscriber(t, persister = createPersister({ type: 'none', memory: { maxItems: 1000 } })) {
	const tracker = createTracker(persister);
	const server = createServer(tracker, staticPath);
	const httpUrl = await startServer(t, server);
	return { tracker, server, httpUrl, url: httpUrl.replace('http:', 'ws:') + '/subscribe' };
}

async function connect(t, url, options) {
	const socket = new WebSocket(url, options);
	socket.on('error', () => {});
	const messages = [];
	socket.on('message', data => messages.push(JSON.parse(data.toString())));
	t.after(() => socket.terminate());
	await once(socket, 'open', deadline());
	return { socket, messages, send: message => socket.send(JSON.stringify(message)) };
}

test('subscriber serves health checks and its browser client', async t => {
	const { httpUrl } = await createSubscriber(t);
	assert.equal(await (await fetch(httpUrl + '/ping')).text(), 'subscriber server up');
	assert.equal(await (await fetch(httpUrl + '/recency-tracker.js')).text(), clientSource);
	assert.equal((await fetch(httpUrl + '/subscribe/info')).status, 404);
});

test('subscriber only accepts WebSocket upgrades at /subscribe', async t => {
	const { url } = await createSubscriber(t);
	await assert.rejects(connect(t, url + '/unexpected'), /Unexpected server response: 400/);
});

test('subscriber sends existing and newly published resource versions', async t => {
	const { tracker, url } = await createSubscriber(t);
	tracker.publish('/test/a', '2', { doer: 'earlier' });
	const { send, messages } = await connect(t, url);
	send({ type: 'subscribe', resource: '/test/a', version: 1 });
	await waitFor(() => messages.length === 1);
	assert.deepEqual(messages[0], { type: 'update', resource: '/test/a', version: '2', data: { doer: 'earlier' } });
	tracker.publish('/test/a', '3', { doer: 'later' });
	await waitFor(() => messages.length === 2);
	assert.deepEqual(messages[1], { type: 'update', resource: '/test/a', version: '3', data: { doer: 'later' } });
});

test('unsubscribe stops updates for that resource while preserving other subscriptions', async t => {
	const { tracker, url } = await createSubscriber(t);
	const { send, messages } = await connect(t, url);
	send({ type: 'subscribe', resource: '/test/a', version: '0' });
	send({ type: 'subscribe', resource: '/test/b', version: '0' });
	await waitFor(() => Object.keys(tracker.waitingClientsByResource['/test/b'] || {}).length === 1);
	send({ type: 'unsubscribe', resource: '/test/a' });
	await waitFor(() => Object.keys(tracker.waitingClientsByResource['/test/a']).length === 0);
	tracker.publish('/test/a', '1', {});
	tracker.publish('/test/b', '1', {});
	await waitFor(() => messages.length > 0);
	assert.deepEqual(messages, [{ type: 'update', resource: '/test/b', version: '1', data: {} }]);
});

test('invalid subscription messages do not allocate tracker clients or break the connection', async t => {
	const { tracker, url } = await createSubscriber(t);
	const { socket, send, messages } = await connect(t, url);
	for (const message of [null, [], 1, false, {},
		{ type: 'unknown', resource: '/test/a', version: '1' },
		{ type: 'unsubscribe', resource: '/test/a' },
		{ type: 'subscribe', resource: 'constructor', version: '1' },
		{ type: 'subscribe', resource: '/test/a' },
		{ type: 'subscribe', resource: '/test/a', version: {} }]) {
		send(message);
	}
	const pong = once(socket, 'pong', deadline());
	socket.ping();
	await pong;
	assert.deepEqual(Object.keys(tracker.waitingClientsById), []);
	tracker.publish('/test/a', '2', {});
	send({ type: 'subscribe', resource: '/test/a', version: '1' });
	await waitFor(() => messages.length === 1);
});

for (const request of [
	{ name: 'malformed JSON', data: '{', code: 1007 },
	{ name: 'binary messages', data: Buffer.from('{}'), code: 1003 },
	{ name: 'messages larger than 64 KiB', data: 'x'.repeat(64 * 1024 + 1), code: 1009 }
]) {
	test(`subscriber closes connections sending ${request.name} and removes their subscriptions`, async t => {
		const { tracker, url } = await createSubscriber(t);
		const { socket, send } = await connect(t, url);
		send({ type: 'subscribe', resource: '/test/a', version: '1' });
		await waitFor(() => Object.keys(tracker.waitingClientsById).length === 1);
		const closed = once(socket, 'close', deadline());
		socket.send(request.data);
		assert.equal((await closed)[0], request.code);
		await waitFor(() => Object.keys(tracker.waitingClientsById).length === 0);
		assert.deepEqual(tracker.waitingClientsByResource['/test/a'], {});
	});
}

test('abrupt disconnects remove tracker subscriptions', async t => {
	const { tracker, url } = await createSubscriber(t);
	const { socket, send } = await connect(t, url);
	send({ type: 'subscribe', resource: '/test/a', version: '1' });
	await waitFor(() => Object.keys(tracker.waitingClientsById).length === 1);
	socket.terminate();
	await waitFor(() => Object.keys(tracker.waitingClientsById).length === 0);
	assert.deepEqual(tracker.waitingClientsByResource['/test/a'], {});
});

test('heartbeats close unresponsive connections and preserve responsive ones', async t => {
	const interval = global.setInterval;
	t.mock.method(global, 'setInterval', callback => interval(callback, 40));
	const { tracker, url } = await createSubscriber(t);
	const { socket, send } = await connect(t, url, { autoPong: false });
	send({ type: 'subscribe', resource: '/test/a', version: '1' });
	await once(socket, 'ping', deadline());
	await once(socket, 'close', deadline());
	await waitFor(() => Object.keys(tracker.waitingClientsById).length === 0);
	const healthy = await connect(t, url);
	await once(healthy.socket, 'ping', deadline());
	await once(healthy.socket, 'ping', deadline());
	tracker.publish('/test/a', '2', {});
	healthy.send({ type: 'subscribe', resource: '/test/a', version: '1' });
	await waitFor(() => healthy.messages.length === 1);
});

test('closing the subscriber server also closes active WebSocket connections', async t => {
	const { server, url } = await createSubscriber(t);
	const { socket } = await connect(t, url);
	const closed = once(socket, 'close', deadline());
	server.close();
	await closed;
});

test('browser client receives HTTP publications and catches up from filesystem storage after reconnecting', async t => {
	const storagePath = await fs.mkdtemp(path.join(os.tmpdir(), 'recency-websocket-test-'));
	t.after(() => fs.rm(storagePath, { recursive: true, force: true }));
	const config = { type: 'filesystem', memory: { maxItems: 1000 }, filesystem: { storagePath } };
	const { tracker, httpUrl } = await createSubscriber(t, createPersister(config));
	const publisher = require('../lib/server/publisher.js').createServer(tracker, 'test-secret');
	const publisherUrl = await startServer(t, publisher);
	const publish = async version => {
		const response = await fetch(publisherUrl + '/publish', {
			method: 'POST',
			body: new URLSearchParams({ secret: 'test-secret', resource: '/test/a', version, data: '{"doer":"test-user"}' }),
			...deadline()
		});
		assert.deepEqual(await response.json(), { ok: true });
	};

	// Use the platform WebSocket implementation to exercise the actual browser-facing API.
	const browser = vm.createContext({ WebSocket: global.WebSocket, URL });
	browser.window = browser;
	browser.location = { href: 'http://example.test/page' };
	let reconnect;
	browser.setTimeout = callback => { reconnect = callback; return 1; };
	browser.clearTimeout = () => { reconnect = null; };
	vm.runInContext(clientSource, browser);
	const client = new browser.RecencyTracker(httpUrl + '/subscribe');
	t.after(() => client.unsubscribe('/test/a'));
	const updates = [];
	client.subscribe('/test/a', '0', (version, data) => updates.push({ version, data }));
	await waitFor(() => Object.keys(tracker.waitingClientsById).length === 1);
	await publish('1');
	await waitFor(() => updates.length === 1);
	assert.equal(updates[0].version, '1');
	assert.equal(updates[0].data.doer, 'test-user');
	client._sock.close();
	await waitFor(() => Object.keys(tracker.waitingClientsById).length === 0 && reconnect);
	await publish('2');
	const resourcePath = tracker.persister.persisters[1].getResourcePath('/test/a');
	await waitFor(() => {
		try {
			return JSON.parse(require('node:fs').readFileSync(resourcePath, 'utf8')).version === '2';
		} catch (error) {
			return false;
		}
	});
	tracker.persister = createPersister(config);
	reconnect();
	await waitFor(() => updates.length === 2);
	assert.equal(updates[1].version, '2');
	client.unsubscribe('/test/a');
	await waitFor(() => Object.keys(tracker.waitingClientsById).length === 0);
	const freshPersister = createPersister(config);
	const persisted = await new Promise(resolve => freshPersister.get('/test/a', resolve));
	assert.deepEqual(persisted, { version: '2', data: { doer: 'test-user' } });
});
