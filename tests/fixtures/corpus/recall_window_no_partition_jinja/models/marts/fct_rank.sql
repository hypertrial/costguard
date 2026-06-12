select rank() over (order by {{ adapter.quote('score') }}) as rnk from events
