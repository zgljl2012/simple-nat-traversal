from flask import Flask, request, jsonify

app = Flask(__name__)

@app.route("/", methods=['GET', 'POST'])
def hello_world():
	if request.method == 'POST':
		return request.data
	return "<p>Hello, World!</p>"
