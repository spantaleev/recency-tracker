var Persister = function (persisters) {
	if (persisters.length === 0) {
		throw new Error('The chained persister is meant to be used with at least one persister.');
	}

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
		failedPersisters = [],
		currentPersister = null,
		self = this;

	var tryNext = function () {
		if (currentIdx === self.persisters.length) {
			//No more to try. Missed everywhere.
			cb(null);
			return;
		}

		currentPersister = self.persisters[currentIdx];

		currentIdx += 1;

		currentPersister.get(resource, onPersisterResponse);
	};

	var populateFailedPersisters = function (data) {
		failedPersisters.forEach(function (persister) {
			persister.set(resource, data);
		});
	};

	var onPersisterResponse = function (data) {
		if (data !== null) {
			populateFailedPersisters(data);
			cb(data);
		} else {
			failedPersisters.push(currentPersister);
			tryNext();
		}
	};

	tryNext();
};

module.exports.Persister = Persister;
