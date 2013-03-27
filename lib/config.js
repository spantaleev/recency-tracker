const
	path = require('path');
	fs = require('fs');

var configPath = path.join(process.cwd(), 'config', 'config.json');
if (fs.existsSync(configPath)) {
	var configJson = fs.readFileSync(configPath);
	try {
		module.exports = JSON.parse(configJson);
	} catch (e) {
		console.error('Cannot parse configuration (' + configPath + '): ' + e.message);
		process.exit(1);
	}
} else {
	console.error('Missing config from: ' + configPath);
	process.exit(1);
}
