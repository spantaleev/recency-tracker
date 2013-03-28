var Persister = function (config) {
	this.config = config;
};

Persister.prototype.set = function (resource, data, cb) {
	if (cb) {
		cb();
	}
};

Persister.prototype.get = function (resource, cb) {
	cb(null);
};

module.exports.Persister = Persister;

