(component
  (type $ui-instance
    (instance
      (type (;0;) (record (field "id" string) (field "value" string)))
      (export (;1;) "field-value" (type (eq 0)))
      (type (;2;) (list 1))
      (type (;3;) (record (field "view-id" string) (field "action-id" string) (field "fields" 2)))
      (export (;4;) "view-action-event" (type (eq 3)))
      (type (;5;) (record (field "extension-id" string) (field "command-id" string) (field "node-id" string) (field "node-name" string) (field "node-type" string) (field "database-type" string) (field "connection-id" string)))
      (export (;6;) "action-context" (type (eq 5)))
      (type (;7;) (enum "info" "success" "warning" "error"))
      (export (;8;) "notification-level" (type (eq 7)))
      (type (;9;) (func (param "level" 8) (param "title" string) (param "message" string)))
      (export (;0;) "notify" (func (type 9)))
      (type (;10;) (option 6))
      (type (;11;) (func (result 10)))
      (export (;1;) "current-action-context" (func (type 11)))
      (type (;12;) (func (param "title" string) (param "payload" string)))
      (export (;2;) "open-result-view" (func (type 12)))
      (type (;13;) (func (param "connection-id" string)))
      (export (;3;) "refresh-tree" (func (type 13)))
    )
  )
  (import "onet:extension/ui" (instance $ui (type $ui-instance)))
  (alias export $ui "view-action-event" (type $view-action-event))
  (type $handle-view-action-ty (func (param "event" $view-action-event)))
  (type $db-instance
    (instance
      (type (;0;) (option string))
      (type (;1;) (record (field "id" string) (field "name" string) (field "driver" string) (field "database" 0)))
      (export (;2;) "connection-info" (type (eq 1)))
      (type (;3;) (list 2))
      (type (;4;) (record (field "code" string) (field "message" string)))
      (export (;5;) "db-error" (type (eq 4)))
      (type (;6;) (result 3 (error 5)))
      (type (;7;) (func (result 6)))
      (export (;0;) "list-connections" (func (type 7)))
    )
  )
  (import "onet:extension/db" (instance $db (type $db-instance)))
  (type $task-instance
    (instance
      (type (enum "running" "completed" "failed" "cancelled"))
      (export "task-state" (type (eq 0)))
      (type (option string))
      (type (record (field "id" string) (field "title" string) (field "state" 1) (field "message" 2)))
      (export "task-status" (type (eq 3)))
      (type (func (param "status" 4)))
      (export "report-status" (func (type 5)))
      (type (func (param "task-id" string) (result bool)))
      (export "is-cancelled" (func (type 6)))
    )
  )
  (import "onet:extension/task" (instance $task (type $task-instance)))
  (core module $memory
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 4096))
    (data (i32.const 16) "coverage")
    (data (i32.const 32) "component smoke")
    (data (i32.const 64) "conn1")
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (local $result i32)
      global.get $heap
      local.get 2
      i32.const 1
      i32.sub
      i32.add
      local.get 2
      i32.const 1
      i32.sub
      i32.const -1
      i32.xor
      i32.and
      local.tee $result
      local.get 3
      i32.add
      global.set $heap
      local.get $result)
  )
  (core instance $mem (instantiate $memory))
  (alias core export $mem "memory" (core memory $memory))
  (alias core export $mem "cabi_realloc" (core func $realloc))
  (alias export $db "list-connections" (func $list-connections))
  (alias export $ui "notify" (func $notify))
  (alias export $ui "current-action-context" (func $current-action-context))
  (alias export $ui "open-result-view" (func $open-result-view))
  (alias export $ui "refresh-tree" (func $refresh-tree))
  (alias export $task "is-cancelled" (func $is-cancelled))
  (core func $list-connections-core (canon lower (func $list-connections) (memory $memory) (realloc $realloc) string-encoding=utf8))
  (core func $notify-core (canon lower (func $notify) (memory $memory) (realloc $realloc) string-encoding=utf8))
  (core func $current-action-context-core (canon lower (func $current-action-context) (memory $memory) (realloc $realloc) string-encoding=utf8))
  (core func $open-result-view-core (canon lower (func $open-result-view) (memory $memory) (realloc $realloc) string-encoding=utf8))
  (core func $refresh-tree-core (canon lower (func $refresh-tree) (memory $memory) (realloc $realloc) string-encoding=utf8))
  (core func $is-cancelled-core (canon lower (func $is-cancelled) (memory $memory) (realloc $realloc) string-encoding=utf8))
  (core instance $imports
    (export "list-connections" (func $list-connections-core))
    (export "notify" (func $notify-core))
    (export "current-action-context" (func $current-action-context-core))
    (export "open-result-view" (func $open-result-view-core))
    (export "refresh-tree" (func $refresh-tree-core))
    (export "is-cancelled" (func $is-cancelled-core)))
  (core module $m
    (type $ret (func (param i32)))
    (type $notify (func (param i32 i32 i32 i32 i32)))
    (type $strings (func (param i32 i32 i32 i32)))
    (type $string (func (param i32 i32)))
    (type $cancel (func (param i32 i32) (result i32)))
    (import "imports" "list-connections" (func $list-connections (type $ret)))
    (import "imports" "notify" (func $notify (type $notify)))
    (import "imports" "current-action-context" (func $current-action-context (type $ret)))
    (import "imports" "open-result-view" (func $open-result-view (type $strings)))
    (import "imports" "refresh-tree" (func $refresh-tree (type $string)))
    (import "imports" "is-cancelled" (func $is-cancelled (type $cancel)))
    (func $exercise
      i32.const 512
      call $list-connections
      i32.const 1024
      call $current-action-context
      i32.const 0
      i32.const 16
      i32.const 8
      i32.const 32
      i32.const 15
      call $notify
      i32.const 16
      i32.const 8
      i32.const 32
      i32.const 15
      call $open-result-view
      i32.const 64
      i32.const 5
      call $refresh-tree
      i32.const 16
      i32.const 8
      call $is-cancelled
      drop)
    (func (export "activate") call $exercise)
    (func (export "run-action") call $exercise)
    (func (export "handle-view-action") (param i32 i32 i32 i32 i32 i32))
    (func (export "deactivate"))
  )
  (core instance $i (instantiate $m (with "imports" (instance $imports))))
  (func $activate (canon lift (core func $i "activate")))
  (func $run-action (canon lift (core func $i "run-action")))
  (func $handle-view-action (type $handle-view-action-ty) (canon lift (core func $i "handle-view-action") (memory $memory) (realloc $realloc) string-encoding=utf8))
  (func $deactivate (canon lift (core func $i "deactivate")))
  (export "activate" (func $activate))
  (export "run-action" (func $run-action))
  (export "handle-view-action" (func $handle-view-action) (func (type $handle-view-action-ty)))
  (export "deactivate" (func $deactivate))
)
