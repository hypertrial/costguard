select id from events where created_at >= dateadd(day, -7, current_date)
