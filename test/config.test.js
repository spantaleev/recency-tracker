const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const configModule = require.resolve('../lib/config.js');
const config = { publisher: { secret: 'file-secret' }, subscriber: { port: 9999 } };

function fixture(t) {
	const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'recency-tracker-config-'));
	t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
	fs.mkdirSync(path.join(directory, 'config'));
	fs.writeFileSync(path.join(directory, 'config', 'config.json'), JSON.stringify(config));
	return directory;
}

function load(cwd, overrides = {}) {
	const env = { ...process.env };
	delete env.RECENCY_TRACKER_CONFIG;
	delete env.RECENCY_TRACKER_PUBLISHER_SECRET;
	return spawnSync(process.execPath, ['-e', `console.log(JSON.stringify(require(${JSON.stringify(configModule)})))`], {
		cwd,
		env: { ...env, ...overrides },
		encoding: 'utf8',
		timeout: 5000
	});
}

test('configuration defaults to config/config.json in the working directory', t => {
	const result = load(fixture(t));
	assert.equal(result.status, 0, result.stderr);
	assert.deepEqual(JSON.parse(result.stdout), config);
});

test('configuration can be mounted outside the working directory', t => {
	const directory = fixture(t);
	const mountedConfig = path.join(directory, 'mounted.json');
	fs.writeFileSync(mountedConfig, JSON.stringify({ ...config, subscriber: { port: 8888 } }));
	const result = load(directory, { RECENCY_TRACKER_CONFIG: mountedConfig });
	assert.equal(result.status, 0, result.stderr);
	assert.equal(JSON.parse(result.stdout).subscriber.port, 8888);
});

test('environment secret overrides the file literally without modifying it', t => {
	const directory = fixture(t);
	const secret = 'slashes/and&backslashes\\with$quotes"\'and\nnewlines';
	const result = load(directory, { RECENCY_TRACKER_PUBLISHER_SECRET: secret });
	assert.equal(result.status, 0, result.stderr);
	assert.deepEqual(JSON.parse(result.stdout), { ...config, publisher: { secret } });
	assert.deepEqual(JSON.parse(fs.readFileSync(path.join(directory, 'config', 'config.json'))), config);
});

test('an explicitly empty environment secret fails startup', t => {
	const result = load(fixture(t), { RECENCY_TRACKER_PUBLISHER_SECRET: '' });
	assert.notEqual(result.status, 0);
	assert.match(result.stderr, /RECENCY_TRACKER_PUBLISHER_SECRET must not be empty/);
});

test('missing configuration reports the selected path', t => {
	const directory = fixture(t);
	const missingPath = path.join(directory, 'missing.json');
	const result = load(directory, { RECENCY_TRACKER_CONFIG: missingPath });
	assert.notEqual(result.status, 0);
	assert.ok(result.stderr.includes('Missing config from: ' + missingPath));
});

test('invalid configuration reports a parse error and the selected path', t => {
	const directory = fixture(t);
	const configPath = path.join(directory, 'invalid.json');
	fs.writeFileSync(configPath, '{');
	const result = load(directory, { RECENCY_TRACKER_CONFIG: configPath });
	assert.notEqual(result.status, 0);
	assert.ok(result.stderr.includes('Cannot parse configuration (' + configPath + ')'));
});
