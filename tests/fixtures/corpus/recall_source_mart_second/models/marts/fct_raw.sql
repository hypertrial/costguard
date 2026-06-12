select id from {{ source('raw', 'events') }}
