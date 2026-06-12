select id from events where date(block_time) = {{ var('run_date') }}
