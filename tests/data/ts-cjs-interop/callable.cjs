function request(value) { return { value, marker: request.marker }; }
request.marker = 42;
module.exports = request;
