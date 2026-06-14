select distinct user_id from {{ ref('stg_orders') }} cross join unnest(tag_list) as tag
