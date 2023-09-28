; ModuleID = 'RuSk_codegen'
source_filename = "RuSk_codegen"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

%Vec = type { ptr, i64 }

@0 = private unnamed_addr constant [4 x i8] c"%d\0A\00", align 1
@1 = private unnamed_addr constant [4 x i8] c"%d\0A\00", align 1

define i32 @main() {
  %variable = alloca %Vec, align 8
  %1 = call ptr @malloc(i64 16)
  %2 = getelementptr inbounds i64, ptr %variable, i32 1
  store i64 2, ptr %2, align 8
  %3 = getelementptr inbounds %Vec, ptr %variable, i32 0
  store ptr %1, ptr %3, align 8
  store [2 x i64] [i64 2, i64 5], ptr %1, align 8
  %4 = getelementptr inbounds %Vec, ptr %variable, i32 0, i32 0
  %5 = load ptr, ptr %4, align 8
  %6 = getelementptr inbounds i64, ptr %5, i64 1
  %7 = load i64, ptr %6, align 8
  %8 = call i64 (ptr, ...) @printf(ptr @0, i64 %7)
  %9 = getelementptr inbounds %Vec, ptr %variable, i32 0, i32 0
  %10 = load ptr, ptr %9, align 8
  store [2 x i64] [i64 2, i64 3], ptr %10, align 8
  %11 = getelementptr inbounds %Vec, ptr %variable, i32 0, i32 0
  %12 = load ptr, ptr %11, align 8
  %13 = getelementptr inbounds i64, ptr %12, i64 1
  %14 = load i64, ptr %13, align 8
  %15 = call i64 (ptr, ...) @printf(ptr @1, i64 %14)
  ret i32 0
}

declare i64 @printf(ptr %0, ...)

declare ptr @malloc(i64 %0)

