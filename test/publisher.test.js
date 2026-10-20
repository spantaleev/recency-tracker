const assert = require('node:assert/strict');
const { once } = require('node:events');
const test = require('node:test');
const { createServer } = require('../lib/server/publisher.js');

async function createPublisher(t) {
	const publications = [];
	const server = createServer({
		publish(...args) {
			publications.push(args);
		}
	}, 'test-secret');

	t.after(() => new Promise((resolve, reject) => {
		server.close(error => error ? reject(error) : resolve());
		server.closeAllConnections();
	}));
	server.listen(0, '127.0.0.1');
	await once(server, 'listening');

	return {
		publications,
		post(body, headers) {
			return fetch(`http://127.0.0.1:${server.address().port}/publish`, {
				method: 'POST',
				body,
				headers,
				signal: AbortSignal.timeout(3000)
			});
		}
	};
}

const validFields = { secret: 'test-secret', resource: '/test/resource', version: '2' };

const invalidRequests = [
	{
		name: 'empty requests',
		error: 'bad request'
	},
	{
		name: 'unparsed request bodies',
		body: JSON.stringify(validFields),
		headers: { 'content-type': 'application/json' },
		error: 'bad request'
	},
	{
		name: 'requests missing required fields',
		body: new URLSearchParams({ secret: 'test-secret', resource: '/test/resource' }),
		error: 'bad request'
	},
	{
		name: 'incorrect publishing secrets',
		body: new URLSearchParams({ ...validFields, secret: 'wrong' }),
		error: 'bad secret'
	},
	{
		name: 'invalid JSON in additional data',
		body: new URLSearchParams({ ...validFields, data: '{' }),
		error: 'bad data - invalid json'
	}
];

for (const request of invalidRequests) {
	test(`publisher rejects ${request.name} without publishing`, async t => {
		const { publications, post } = await createPublisher(t);
		const response = await post(request.body, request.headers);

		assert.equal(response.status, 200);
		assert.deepEqual(await response.json(), { ok: false, error: request.error });
		assert.deepEqual(publications, []);
	});
}

test('publisher accepts form-encoded updates with additional data', async t => {
	const { publications, post } = await createPublisher(t);
	const data = { doer: 'test-user', nested: { value: 1 } };
	const response = await post(new URLSearchParams({ ...validFields, data: JSON.stringify(data) }));

	assert.equal(response.status, 200);
	assert.deepEqual(await response.json(), { ok: true });
	assert.deepEqual(publications, [['/test/resource', '2', data]]);
});

test('publisher defaults missing additional data to an empty object', async t => {
	const { publications, post } = await createPublisher(t);
	const response = await post(new URLSearchParams(validFields));

	assert.equal(response.status, 200);
	assert.deepEqual(await response.json(), { ok: true });
	assert.deepEqual(publications, [['/test/resource', '2', {}]]);
});
