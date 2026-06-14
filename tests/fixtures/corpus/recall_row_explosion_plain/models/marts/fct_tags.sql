select distinct user_id from orders cross join unnest(tag_list) as tag
