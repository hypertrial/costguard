select campaign_id, count(distinct user_id) from events group by 1
