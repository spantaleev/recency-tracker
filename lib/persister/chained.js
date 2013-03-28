var Persister = function (persisters) {
	this.persisters = persisters;
};

Persister.prototype.set = function (resource, data, cb) {
	var finishedCount = 0,
		self = this;

	var onPersisterSet = function () {
		finishedCount += 1;

		if (finishedCount === self.persisters.length) {
			if (cb)  {
				cb();
			}
		}
	};

	this.persisters.forEach(function (persister) {
		persister.set(resource, data, onPersisterSet);
	});
};

Persister.prototype.get = function (resource, cb) {
	var currentIdx = 0,
		self = this;

	var tryNext = function () {
		if (currentIdx === self.persisters.length) {
			//No more to try. Missed everywhere.
			cb(null);
			return;
		}

		var persister = self.persisters[currentIdx];

		currentIdx += 1;

		persister.get(resource, onPersisterResponse);
	};

	var onPersisterResponse = function (data) {
		if (data !== null) {
			cb(data);
		} else {
			tryNext();
		}
	};

	tryNext();
};

module.exports.Persister = Persister;
