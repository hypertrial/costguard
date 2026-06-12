select id, parse_json(payload):event_id::string as event_id from events
