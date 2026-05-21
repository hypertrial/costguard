select id, event_date from {{ source('raw', 'events') }}
