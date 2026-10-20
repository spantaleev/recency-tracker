const
	path = require('path'),
	fs = require('fs');

const configPath = process.env.RECENCY_TRACKER_CONFIG || path.join(process.cwd(), 'config', 'config.json');
if (fs.existsSync(configPath)) {
	var configJson = fs.readFileSync(configPath);
	try {
		module.exports = JSON.parse(configJson);
	} catch (e) {
		throw new Error('Cannot parse configuration (' + configPath + '): ' + e.message);
	}
} else {
	throw new Error('Missing config from: ' + configPath);
}

if (process.env.RECENCY_TRACKER_PUBLISHER_SECRET !== undefined) {
	if (!process.env.RECENCY_TRACKER_PUBLISHER_SECRET) {
		throw new Error('RECENCY_TRACKER_PUBLISHER_SECRET must not be empty');
	}
	module.exports.publisher.secret = process.env.RECENCY_TRACKER_PUBLISHER_SECRET;
}
