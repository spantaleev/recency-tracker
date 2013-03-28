var createMemoryPersister = function () {
	return new (require('./persister/memory.js').Persister)();
};

var createUserPersister = function (config) {
	var modulePath = './persister/' + config.type + '.js';
	if (typeof(config[config.type]) === 'undefined') {
		console.error('Missing configuration for persister: ' + config.type);
		process.exit(1);
	}

	var persisterConfig = config[config.type];

	return new (require(modulePath).Persister)(persisterConfig);
};

module.exports = function (config) {
	var persisters = [createMemoryPersister()];

	if (config.type !== null && config.type !== 'none') {
		persisters.push(createUserPersister(config));
	}

	return new (require('./persister/chained.js').Persister)(persisters);
};
