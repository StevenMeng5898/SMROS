/*****************************************************************************
 *  The BYTE UNIX Benchmarks - Release 3
 *          Module: dhry.h   SID: 3.4 5/15/91 19:30:21
 *
 *****************************************************************************
 * Bug reports, patches, comments, suggestions should be sent to:
 *
 *      Ben Smith, Rick Grehan or Tom Yager
 *      ben@bytepb.byte.com   rick_g@bytepb.byte.com   tyager@bytepb.byte.com
 *
 *****************************************************************************
 *  Modification Log:
 *  addapted from:
 *
 *
 *                   "DHRYSTONE" Benchmark Program
 *                   -----------------------------
 *
 *  Version:    C, Version 2.1
 *
 *  File:       dhry.h (part 1 of 3)
 *
 *  Date:       May 25, 1988
 *
 *  Author:     Reinhold P. Weicker
 *
 *  SMROS port:
 *              The original Dhrystone data types and benchmark procedures are
 *              kept in C.  The libc and UNIX timer dependencies are replaced by
 *              the small SMROS syscall shim in smros_dhry.c.
 *
 ***************************************************************************
 */

#ifndef DHRY_H
#define DHRY_H

#define Mic_secs_Per_Second 1000000.0

#ifdef NOSTRUCTASSIGN
#define structassign(d, s) memcpy(&(d), &(s), sizeof(d))
#else
#define structassign(d, s) d = s
#endif

#ifdef NOENUM
#define Ident_1 0
#define Ident_2 1
#define Ident_3 2
#define Ident_4 3
#define Ident_5 4
typedef int Enumeration;
#else
typedef enum { Ident_1, Ident_2, Ident_3, Ident_4, Ident_5 } Enumeration;
#endif

#define Null 0
#define true 1
#define false 0

typedef int One_Thirty;
typedef int One_Fifty;
typedef char Capital_Letter;
typedef int Boolean;
typedef char Str_30[31];
typedef int Arr_1_Dim[50];
typedef int Arr_2_Dim[50][50];

typedef struct record {
  struct record* Ptr_Comp;
  Enumeration Discr;
  union {
    struct {
      Enumeration Enum_Comp;
      int Int_Comp;
      char Str_Comp[31];
    } var_1;
    struct {
      Enumeration E_Comp_2;
      char Str_2_Comp[31];
    } var_2;
    struct {
      char Ch_1_Comp;
      char Ch_2_Comp;
    } var_3;
  } variant;
} Rec_Type, *Rec_Pointer;

extern Rec_Pointer Ptr_Glob, Next_Ptr_Glob;
extern int Int_Glob;
extern Boolean Bool_Glob;
extern char Ch_1_Glob, Ch_2_Glob;
extern int Arr_1_Glob[50];
extern int Arr_2_Glob[50][50];
extern volatile unsigned long Run_Index;

uint64_t dhry_run(uint64_t number_of_runs);
int dhry_verify(uint64_t number_of_runs);

#endif
