module.exports = {

	"isValidResourceName": function (name) {
		if (typeof(name) !== 'string') {
			return false;
		}

		//By making sure that resource names are prefixed with "/",
		//we avoid problems with javascript objects being unsafe dictionaries
		//that may people to potentially overwrite internal Object attributes ("prototype", etc.)

		return (name.substr(0, 1) === '/')
	}

};
