var Persister = function (config) {
	this.config = config;
	this.resources = {};
};

Persister.prototype.set = function (resource, data, cb) {
	this.resources[resource] = data;
	if (cb) {
		cb();
	}
};

Persister.prototype.get = function (resource, cb) {
	if (typeof(this.resources[resource]) === 'undefined') {
		cb(null);
	} else {
		cb(this.resources[resource]);
	}
};

module.exports.Persister = Persister;
