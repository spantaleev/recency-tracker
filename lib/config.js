const
	path = require('path');
	fs = require('fs');

var configPath = path.join(process.cwd(), 'config', 'config.json');
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
