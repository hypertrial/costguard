select row_number() over (partition by user_id order by id) as rn from events
