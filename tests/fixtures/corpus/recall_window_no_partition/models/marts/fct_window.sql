select row_number() over (order by id) as rn from events
