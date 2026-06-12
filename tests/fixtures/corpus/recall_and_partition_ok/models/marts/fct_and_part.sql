select id from events where event_date >= current_date and block_time < current_date + interval '1' day
