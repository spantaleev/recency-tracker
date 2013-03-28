module.exports = function (config) {
	if (config.type === null || config.type === 'none') {
		var modulePath = './persistence/none.js',
			persisterConfig = {};
	} else {
		var modulePath = './persistence/' + config.type + '.js';
		if (typeof(config[config.type]) === 'undefined') {
			console.error('Missing configuration for persister: ' + config.type);
			process.exit(1);
		}
		var persisterConfig = config[config.type];
	}

	return new (require(modulePath).Persister)(persisterConfig);
};
