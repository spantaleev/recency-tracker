const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');

const source = fs.readFileSync(path.join(__dirname, '../resources/static/recency-tracker.js'), 'utf8');

function createBrowser() {
	const sockets = [];
	const timers = new Map();
	let nextTimer = 0;
	class WebSocket {
		static OPEN = 1;
		constructor(url) {
			this.url = url;
			this.readyState = 0;
			this.sent = [];
			sockets.push(this);
		}
		open() {
			this.readyState = 1;
			if (this.onopen) this.onopen();
		}
		send(message) {
			assert.equal(this.readyState, 1, 'Cannot send before the connection is open');
			this.sent.push(JSON.parse(message));
		}
		close() {
			this.readyState = 3;
			if (this.onclose) this.onclose();
		}
		receive(message) {
			if (this.onmessage) this.onmessage({ data: JSON.stringify(message) });
		}
	}
	const browser = vm.createContext({ WebSocket, URL });
	browser.window = browser;
	browser.location = { href: 'https://example.test/page' };
	browser.setTimeout = (callback, delay) => {
		assert.equal(delay, 5000);
		timers.set(++nextTimer, callback);
		return nextTimer;
	};
	browser.clearTimeout = id => timers.delete(id);
	vm.runInContext(source, browser);
	return {
		RecencyTracker: browser.RecencyTracker,
		sockets,
		timers,
		reconnect() {
			assert.equal(timers.size, 1);
			const [id, callback] = timers.entries().next().value;
			timers.delete(id);
			callback();
		}
	};
}

test('browser accepts WebSocket, HTTP, and relative subscription URLs without opening idle connections', () => {
	for (const [url, expected] of [
		['ws://tracker.test/subscribe', 'ws://tracker.test/subscribe'],
		['wss://tracker.test/subscribe', 'wss://tracker.test/subscribe'],
		['http://tracker.test/subscribe', 'ws://tracker.test/subscribe'],
		['https://tracker.test/subscribe', 'wss://tracker.test/subscribe'],
		['https://tracker.test/recency-tracker/subscribe', 'wss://tracker.test/recency-tracker/subscribe'],
		['/subscribe', 'wss://example.test/subscribe']
	]) {
		const browser = createBrowser();
		const tracker = new browser.RecencyTracker(url);
		assert.equal(browser.sockets.length, 0);
		tracker.subscribe('/test/a', 1, () => {});
		assert.equal(browser.sockets[0].url, expected);
	}
	const browser = createBrowser();
	for (const url of ['ftp://tracker.test/subscribe', 'wss://tracker.test/subscribe#fragment']) {
		assert.throws(() => new browser.RecencyTracker(url), /Expected a WebSocket or HTTP URL/);
	}
});

test('browser announces each subscription once and supports multiple callbacks per resource', () => {
	const browser = createBrowser();
	const tracker = new browser.RecencyTracker('/subscribe');
	const updates = [];
	tracker.subscribe('/test/a', 1, version => updates.push('first:' + version));
	tracker.subscribe('/test/a', 1, version => updates.push('second:' + version));
	tracker.subscribe('/test/b', 4, () => {});
	assert.equal(browser.sockets.length, 1);
	const socket = browser.sockets[0];
	assert.deepEqual(socket.sent, []);
	socket.open();
	assert.deepEqual(socket.sent, [
		{ type: 'subscribe', resource: '/test/a', version: '1' },
		{ type: 'subscribe', resource: '/test/b', version: '4' }
	]);
	socket.receive({ type: 'update', resource: '/test/a', version: '2', data: {} });
	assert.deepEqual(updates, ['first:2', 'second:2']);
});

test('browser only announces subscriptions that still exist when a connection opens', () => {
	const browser = createBrowser();
	const tracker = new browser.RecencyTracker('/subscribe');
	tracker.subscribe('/test/a', 1, () => {});
	tracker.subscribe('/test/b', 2, () => {});
	tracker.unsubscribe('/test/a');
	browser.sockets[0].open();
	assert.deepEqual(browser.sockets[0].sent, [{ type: 'subscribe', resource: '/test/b', version: '2' }]);
});

test('browser sends unsubscriptions and disconnects after the last resource is removed', () => {
	const browser = createBrowser();
	const tracker = new browser.RecencyTracker('/subscribe');
	let updates = 0;
	tracker.subscribe('/test/a', 1, () => updates++);
	browser.sockets[0].open();
	tracker.subscribe('/test/b', 1, () => {});
	tracker.unsubscribe('/test/a');
	assert.deepEqual(browser.sockets[0].sent.slice(1), [
		{ type: 'subscribe', resource: '/test/b', version: '1' },
		{ type: 'unsubscribe', resource: '/test/a' }
	]);
	browser.sockets[0].receive({ type: 'update', resource: '/test/a', version: '2', data: {} });
	assert.equal(updates, 0);
	tracker.unsubscribe('/test/b');
	tracker.unsubscribe('/test/missing');
	assert.equal(browser.sockets[0].readyState, 3);
	assert.equal(browser.timers.size, 0);
});

test('browser reconnects using the last received version without replaying stale subscription commands', () => {
	const browser = createBrowser();
	const tracker = new browser.RecencyTracker('/subscribe');
	tracker.subscribe('/test/a', 1, () => {});
	const first = browser.sockets[0];
	first.open();
	first.receive({ type: 'update', resource: '/test/a', version: '2', data: {} });
	const staleClose = first.onclose;
	first.close();
	tracker.subscribe('/test/b', 3, () => {});
	tracker.unsubscribe('/test/b');
	assert.equal(browser.sockets.length, 1);
	browser.reconnect();
	browser.sockets[1].open();
	assert.deepEqual(browser.sockets[1].sent, [{ type: 'subscribe', resource: '/test/a', version: '2' }]);
	staleClose();
	assert.equal(browser.timers.size, 0);
});

test('browser cancels pending reconnects when its last subscription is removed', () => {
	const browser = createBrowser();
	const tracker = new browser.RecencyTracker('/subscribe');
	tracker.subscribe('/test/a', 1, () => {});
	browser.sockets[0].open();
	browser.sockets[0].close();
	assert.equal(browser.timers.size, 1);
	tracker.unsubscribe('/test/a');
	assert.equal(browser.timers.size, 0);
	tracker.subscribe('/test/a', 2, () => {});
	browser.sockets[1].open();
	assert.deepEqual(browser.sockets[1].sent, [{ type: 'subscribe', resource: '/test/a', version: '2' }]);
});

test('browser can unsubscribe during connection setup and later create a fresh connection', () => {
	const browser = createBrowser();
	const tracker = new browser.RecencyTracker('/subscribe');
	tracker.subscribe('/test/a', 1, () => {});
	const staleOpen = browser.sockets[0].onopen;
	tracker.unsubscribe('/test/a');
	tracker.subscribe('/test/a', 2, () => {});
	staleOpen();
	browser.sockets[1].open();
	assert.deepEqual(browser.sockets[0].sent, []);
	assert.deepEqual(browser.sockets[1].sent, [{ type: 'subscribe', resource: '/test/a', version: '2' }]);
});

test('browser retries failed initial connections', () => {
	const browser = createBrowser();
	const tracker = new browser.RecencyTracker('/subscribe');
	tracker.subscribe('/test/a', 1, () => {});
	browser.sockets[0].close();
	browser.reconnect();
	browser.sockets[1].open();
	assert.deepEqual(browser.sockets[1].sent, [{ type: 'subscribe', resource: '/test/a', version: '1' }]);
});

test('browser ignores malformed and unrelated update messages', () => {
	const browser = createBrowser();
	const tracker = new browser.RecencyTracker('/subscribe');
	const updates = [];
	tracker.subscribe('/test/a', 1, version => updates.push(version));
	const socket = browser.sockets[0];
	socket.open();
	socket.onmessage({ data: '{' });
	for (const message of [null, [], 1, {},
		{ type: 'unknown' },
		{ type: 'update', resource: '/test/missing', version: '2' },
		{ type: 'update', resource: '/test/a' }]) {
		socket.receive(message);
	}
	socket.receive({ type: 'update', resource: '/test/a', version: '2', data: {} });
	assert.deepEqual(updates, ['2']);
});
