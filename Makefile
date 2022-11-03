
simple-server:
	@FLASK_APP=fixtures/server flask run --host=0.0.0.0 --port=5001

cloc:
	@gocloc --not-match-d="target" .
