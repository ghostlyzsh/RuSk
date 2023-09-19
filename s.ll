; ModuleID = 'RuSk_codegen'
source_filename = "RuSk_codegen"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

%Vec = type { ptr, i64 }

@0 = private unnamed_addr constant [4 x i8] c"%d\0A\00", align 1

define i32 @main() {
  %variable = alloca %Vec, align 8
  %1 = call ptr @malloc(i64 4)
  %2 = getelementptr inbounds [4 x i64], ptr %variable, i64 1, i64 0
  store i64 4, ptr %2, align 8
  %3 = getelementptr inbounds [4 x i64], ptr %variable, i64 0, i64 0
  store ptr %1, ptr %3, align 8
  %4 = getelementptr inbounds [4 x i64], ptr %1, i64 0, i64 0
  store i64 1, ptr %4, align 8
  %5 = getelementptr inbounds [4 x i64], ptr %1, i64 0, i64 1
  store i64 2, ptr %5, align 8
  %6 = getelementptr inbounds [4 x i64], ptr %1, i64 0, i64 2
  store i64 3, ptr %6, align 8
  %7 = getelementptr inbounds [4 x i64], ptr %1, i64 0, i64 3
  store i64 4, ptr %7, align 8
  %8 = getelementptr inbounds ptr, ptr %variable, i32 0, i32 0
  %9 = load ptr, ptr %8, align 8
  %10 = getelementptr inbounds [4 x i64], ptr %9, i64 0, i64 3
  %11 = load i64, ptr %10, align 8
  %12 = call i64 (ptr, ...) @printf(ptr @0, i64 %11)
  ret i32 0
}

declare i64 @printf(ptr %0, ...)

declare ptr @malloc(i64 %0)
