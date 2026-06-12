select * from hive.default.events e
join iceberg.analytics.users u on e.user_id = u.id
