select * from hive.default.orders o join iceberg.analytics.users u on o.id = u.id
