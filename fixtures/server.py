from flask import Flask, request, jsonify

app = Flask(__name__)

@app.route("/", methods=['GET', 'POST'])
def hello_world():
	if request.method == 'POST':
		return jsonify(request.get_json())
	return "<p>Hello, World!</p>"
