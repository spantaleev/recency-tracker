const
	crypto = require('crypto'),
	path = require('path'),
	fs = require('fs');

var Persister = function (config) {
	this.config = config;
};

Persister.prototype.getResourcePath = function (resource) {
	var name = crypto.createHash('sha256').update(resource, 'utf8').digest() + '.json';
	return path.join(this.config.storagePath, name);
};

Persister.prototype.set = function (resource, data, cb) {
	var filePath = this.getResourcePath(resource);

	fs.writeFile(filePath, JSON.stringify(data), function (err) {
		if (err) {
			throw err;
		}
		if (cb) {
			cb();
		}
	});
};

Persister.prototype.get = function (resource, cb) {
	var filePath = this.getResourcePath(resource);

	fs.exists(filePath, function (exists) {
		if (!exists) {
			cb(null);
			return;
		}

		fs.readFile(filePath, function (err, data) {
			if (err) {
				throw err;
			}
			cb(JSON.parse(data));
		});
	});
};

module.exports.Persister = Persister;
