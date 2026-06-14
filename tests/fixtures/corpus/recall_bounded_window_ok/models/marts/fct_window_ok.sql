select sum(amount) over (partition by user_id order by ts rows between 29 preceding and current row) from events
