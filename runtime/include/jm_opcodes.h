/* 由 micc --emit-header 生成，请勿手改。 */
#ifndef JM_OPCODES_H
#define JM_OPCODES_H

#define OP_HALT 0
#define OP_CONST 1
#define OP_LOAD_LOCAL 2
#define OP_STORE_LOCAL 3
#define OP_LOAD_GLOBAL 4
#define OP_STORE_GLOBAL 5
#define OP_POP 6
#define OP_DUP 7
#define OP_SWAP 8
#define OP_BINOP 9
#define OP_NEG 10
#define OP_NOT 11
#define OP_CMP 12
#define OP_JUMP 13
#define OP_JUMP_IF_FALSE 14
#define OP_JUMP_IF_TRUE 15
#define OP_JUMP_IF_FALSE_OR_POP 16
#define OP_JUMP_IF_TRUE_OR_POP 17
#define OP_CALL 18
#define OP_RETURN 19
#define OP_BUILD_LIST 20
#define OP_BUILD_DICT 21
#define OP_GET_ITEM 22
#define OP_SET_ITEM 23
#define OP_GET_ITER 24
#define OP_FOR_ITER 25
#define OP_MAKE_FUNCTION 26
#define OP_IMPORT 27
#define OP_MAKE_CLOSURE 28
#define OP_TRY 29
#define OP_TRY_END 30
#define OP_THROW 31
#define OP_NEW_INSTANCE 32
#define OP_GET_FIELD 33
#define OP_SET_FIELD 34
#define OP_CALL_METHOD 35
#define OP_CALL_INIT 36
#define OP_RETURN_METHOD 37
#define OP_TASK_SPAWN 38
#define OP_TASK_YIELD 39
#define OP_TASK_SLEEP 40
#define OP_TASK_WAIT 41
#define OP_TASK_KILL 42

#define BIN_ADD 0
#define BIN_SUB 1
#define BIN_MUL 2
#define BIN_DIV 3
#define BIN_FLOORDIV 4
#define BIN_MOD 5
#define BIN_POW 6

#define CMP_EQ 0
#define CMP_NE 1
#define CMP_LT 2
#define CMP_GT 3
#define CMP_LE 4
#define CMP_GE 5

#define K_INT 0
#define K_STR 1
#define K_NIL 2
#define K_TRUE 3
#define K_FALSE 4
#define K_DEC 5

#endif
