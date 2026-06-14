select user_id, tag from orders cross join unnest(tag_list) as tag
