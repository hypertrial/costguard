select id from events where event_date = {{ var('run_date') }} or block_time >= current_date
