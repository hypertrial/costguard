select regexp_extract({{ adapter.quote('msg') }}, 'a'), regexp_extract({{ adapter.quote('msg') }}, 'b') from logs
