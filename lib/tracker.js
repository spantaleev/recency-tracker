var events = require('events');

var Tracker = function () {
	this.waitingClientsById = {};
	this.waitingClientsByChannel = {};
	this.channelVersions = {};
};

Tracker.prototype.createClient = function (id) {
	console.log('Creating client #' + id);

	var client = {};
	client.id = id;
	client.channels = {};
	client.events = new events.EventEmitter();

	this.waitingClientsById[client.id] = client;

	return client;
};

Tracker.prototype.subscribe = function (client, channel, version) {
	console.log('Request to subscribe client #' + client.id + ' for: ' + channel);

	if (typeof(client.channels[channel]) !== 'undefined') {
		return;
	}

	client.channels[channel] = version;

	if (typeof(this.waitingClientsByChannel[channel]) === 'undefined') {
		this.waitingClientsByChannel[channel] = {};
	}
	this.waitingClientsByChannel[channel][client.id] = client;

	this.catchUpClient(client, channel);
};

Tracker.prototype.unsubscribe = function (client, channel) {
	console.log('Request to unsubscribe client #' + client.id + ' from: ' + channel);

	if (typeof(client.channels[channel]) === 'undefined') {
		//Already unsubscribed to channel.
		return;
	}

	delete this.waitingClientsByChannel[channel][client.id];
	delete client.channels[channel];

	console.log('Client #' + client.id + ' now subscribed to channels: ' + JSON.stringify(client.channels));
};

Tracker.prototype.deleteClient = function (client) {
	console.log('Deleting client for #' + client.id);

	for (var channel in client.channels) {
		this.unsubscribe(client, channel);
	}

	delete this.waitingClientsById[client.id];
};

Tracker.prototype.publish = function (channel, version) {
	console.log('Received publish request for channel ' + channel + ', version: ' + version);
	this.channelVersions[channel] = version;

	if (typeof(this.waitingClientsByChannel[channel]) === 'undefined') {
		//No one waiting yet.
		return;
	}

	var clients = this.waitingClientsByChannel[channel];
	for (var clientIdx in clients) {
		this.catchUpClient(clients[clientIdx], channel);
	}
};

Tracker.prototype.catchUpClient = function (client, channel) {
	var clientVersion = client.channels[channel],
		latestVersion = (typeof(this.channelVersions[channel]) !== 'undefined' ? this.channelVersions[channel] : null);

	if (latestVersion === null || clientVersion >= latestVersion) {
		console.log('Client #' + client.id + ' is up to date (' + clientVersion + ') for channel: ' + channel);
		return;
	}

	client.channels[channel] = latestVersion;
	console.log('Notifying client #' + client.id + ' for version ' + latestVersion + ' on ' + channel);

	client.events.emit('outdated', channel, latestVersion);
};

module.exports.Tracker = Tracker;
