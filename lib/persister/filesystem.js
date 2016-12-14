const
	path = require('path'),
	fs = require('fs');

let md5 = null;
try {
	md5 = require('md5');
} catch (e) {
	throw new Error('The filesystem persister requires the `md5` package (`npm install md5@2.x`).');
}

var Persister = function (config) {
	this.config = config;
};

Persister.prototype.getResourcePath = function (resource) {
	return path.join(this.config.storagePath, md5(resource) + '.json');
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
