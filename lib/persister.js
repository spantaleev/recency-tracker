var createMemoryPersister = function (persisterConfig) {
	return new (require('./persister/memory.js').Persister)(persisterConfig);
};

var createUserPersister = function (config) {
    if (config.type === 'memory') {
		throw new Error('The memory persister is used in any case (as a cache, before other persisters). Try "none" instead.');
	}

	var modulePath = './persister/' + config.type + '.js';
	if (! (config.type in config)) {
		throw new Error('Missing configuration for persister: ' + config.type);
	}

	var persisterConfig = config[config.type];

	return new (require(modulePath).Persister)(persisterConfig);
};

module.exports = function (config) {
	var persisters = [createMemoryPersister(config.memory)];

	if (config.type !== null && config.type !== 'none') {
		persisters.push(createUserPersister(config));
	}

	return new (require('./persister/chained.js').Persister)(persisters);
};
