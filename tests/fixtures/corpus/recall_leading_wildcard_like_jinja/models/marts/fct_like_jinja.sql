-- domain filter: {{ var('domain') }}
select id from users where email like '%@example.com'
