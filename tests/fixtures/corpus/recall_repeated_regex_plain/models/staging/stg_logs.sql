select regexp_extract(msg, 'error'), regexp_replace(msg, 'x', 'y') from logs
