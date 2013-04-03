var Persister = function (config) {
	this.config = config;
	this.resources = {};
	this.resourcesCount = 0;
};

Persister.prototype.gc = function () {
	if (this.resourcesCount > this.config.maxItems) {
		//Simplest GC - just get rid of everything and start fresh.
		this.resources = {};
		this.resourcesCount = 0;
	}
};

Persister.prototype.set = function (resource, data, cb) {
	this.gc();

	this.resources[resource] = data;
	this.resourcesCount += 1;

	if (cb) {
		cb();
	}
};

Persister.prototype.get = function (resource, cb) {
	if (resource in this.resources) {
		cb(this.resources[resource]);
	} else {
		cb(null);
	}
};

module.exports.Persister = Persister;
