// Integration check for a built or pulled image; run separately from npm test.
const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const { createHash, randomUUID } = require('node:crypto');
const { once } = require('node:events');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { setTimeout: delay } = require('node:timers/promises');

const image = process.argv[2];
if (!image) throw new Error('Usage: node scripts/smoke-container.js IMAGE');

const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'recency-tracker-image-'));
const name = 'recency-tracker-test-' + randomUUID();
const storagePath = path.join(directory, 'data');
const configPath = path.join(directory, 'config.json');
const secret = 'container-secret/with&punctuation\\and$quotes"';
const resource = '/container-test';
const data = { example: 'persisted across containers' };
const storedResource = path.join(storagePath, createHash('sha256').update(resource).digest('hex') + '.json');
let socket;

function docker(...args) {
	return execFileSync('docker', args, { encoding: 'utf8', timeout: 60000 }).trim();
}

async function until(check) {
	const deadline = Date.now() + 15000;
	let lastError;
	while (Date.now() < deadline) {
		try { return await check(); } catch (error) { lastError = error; }
		await delay(100);
	}
	throw lastError;
}

async function start(user) {
	docker('run', '--detach', '--name', name, '--read-only', '--cap-drop=ALL',
		'--security-opt=no-new-privileges', ...(user ? ['--user', user] : []),
		'--mount', `type=bind,src=${configPath},dst=/config/config.json,readonly`,
		'--mount', `type=bind,src=${storagePath},dst=/data`,
		'--env', `RECENCY_TRACKER_PUBLISHER_SECRET=${secret}`,
		'--publish', '127.0.0.1::9998', '--publish', '127.0.0.1::9999', image);
	const ports = JSON.parse(docker('inspect', '--format', '{{json .NetworkSettings.Ports}}', name));
	const publisher = 'http://127.0.0.1:' + ports['9998/tcp'][0].HostPort;
	const subscriber = 'http://127.0.0.1:' + ports['9999/tcp'][0].HostPort;
	for (const [endpoint, body] of [[publisher, 'publisher server up'], [subscriber, 'subscriber server up']]) {
		await until(async () => {
			const response = await fetch(endpoint + '/ping', { signal: AbortSignal.timeout(1000) });
			assert.equal(response.status, 200);
			assert.equal(await response.text(), body);
		});
	}
	return { publisher, subscriber };
}

async function subscribe(subscriber, version) {
	socket = new WebSocket(subscriber.replace('http:', 'ws:') + '/subscribe');
	await once(socket, 'open', { signal: AbortSignal.timeout(5000) });
	const update = once(socket, 'message', { signal: AbortSignal.timeout(5000) });
	socket.send(JSON.stringify({ type: 'subscribe', resource, version }));
	return () => update.then(([event]) => JSON.parse(event.data));
}

async function main() {
	fs.chmodSync(directory, 0o755);
	fs.mkdirSync(storagePath, { mode: 0o777 });
	// The second container uses an arbitrary UID, so its mounted storage must be writable.
	fs.chmodSync(storagePath, 0o777);
	fs.copyFileSync(path.join(__dirname, '../config/container.json.dist'), configPath);
	fs.chmodSync(configPath, 0o444);
	assert.equal(docker('image', 'inspect', '--format', '{{.Config.User}}', image), 'node');
	const { publisher, subscriber } = await start();
	const client = await fetch(subscriber + '/recency-tracker.js');
	assert.equal(client.status, 200);
	assert.match(await client.text(), /RecencyTracker/);
	const nextUpdate = await subscribe(subscriber, '0');
	const publish = async publishingSecret => {
		const response = await fetch(publisher + '/publish', {
			method: 'POST',
			body: new URLSearchParams({ secret: publishingSecret, resource, version: '1', data: JSON.stringify(data) }),
			signal: AbortSignal.timeout(5000)
		});
		assert.equal(response.status, 200);
		return response.json();
	};
	assert.deepEqual(await publish('wrong-secret'), { ok: false, error: 'bad secret' });
	assert.deepEqual(await publish(secret), { ok: true });
	const expected = { type: 'update', resource, version: '1', data };
	assert.deepEqual(await nextUpdate(), expected);
	await until(() => assert.deepEqual(JSON.parse(fs.readFileSync(storedResource)), { version: '1', data }));
	// Stop with a live WebSocket to catch PID 1 and upgraded-socket shutdown bugs.
	docker('stop', '--time', '5', name);
	assert.equal(docker('inspect', '--format', '{{.State.ExitCode}}', name), '0');
	docker('rm', name);
	const restarted = await start('12345:12345');
	assert.equal(docker('exec', name, 'node', '-p', 'process.getuid()'), '12345');
	const catchUp = await subscribe(restarted.subscriber, '0');
	assert.deepEqual(await catchUp(), expected);
	const customResource = resource + '/custom-user';
	const response = await fetch(restarted.publisher + '/publish', {
		method: 'POST',
		body: new URLSearchParams({ secret, resource: customResource, version: '2' }),
		signal: AbortSignal.timeout(5000)
	});
	assert.deepEqual(await response.json(), { ok: true });
	const customFile = path.join(storagePath, createHash('sha256').update(customResource).digest('hex') + '.json');
	await until(() => assert.deepEqual(JSON.parse(fs.readFileSync(customFile)), { version: '2', data: {} }));
	console.log('Container smoke check passed: non-root, read-only root/config, publishing, WebSocket updates, restart persistence, custom UID.');
}

main().catch(error => {
	console.error(error);
	try { console.error(docker('logs', name)); } catch {}
	process.exitCode = 1;
}).finally(() => {
	if (socket && socket.readyState === WebSocket.OPEN) socket.close();
	try { docker('rm', '--force', name); } catch {}
	fs.rmSync(directory, { recursive: true, force: true });
});
