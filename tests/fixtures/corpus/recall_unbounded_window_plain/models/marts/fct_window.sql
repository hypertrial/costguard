select sum(amount) over (partition by user_id order by ts rows between unbounded preceding and unbounded following) from events
