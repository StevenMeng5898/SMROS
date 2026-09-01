/* POSIX scheduling extensions used by the SMROS test ABI. */
#ifndef SMROS_POSIX_SCHED_H
#define SMROS_POSIX_SCHED_H 1

/* The glibc sched.h includes this guarded type after declaring its constants.
 * Define the extended POSIX form first while retaining the Linux declarations
 * through include_next below. */
#include_next <time.h>

#ifndef _BITS_TYPES_STRUCT_SCHED_PARAM
# define _BITS_TYPES_STRUCT_SCHED_PARAM 1
struct sched_param
{
  int sched_priority;
  int sched_ss_low_priority;
  struct timespec sched_ss_repl_period;
  struct timespec sched_ss_init_budget;
  int sched_ss_max_repl;
};
#endif

#include_next <sched.h>

#ifndef SCHED_SPORADIC
# define SCHED_SPORADIC 4
#endif
#ifndef SS_REPL_MAX
# define SS_REPL_MAX 10
#endif

/* These options are advertised only when the implementation supplies the
 * corresponding API behavior. */
#ifdef _POSIX_SPORADIC_SERVER
# undef _POSIX_SPORADIC_SERVER
#endif
#define _POSIX_SPORADIC_SERVER 200809L
#ifdef _POSIX_THREAD_SPORADIC_SERVER
# undef _POSIX_THREAD_SPORADIC_SERVER
#endif
#define _POSIX_THREAD_SPORADIC_SERVER 200809L

#endif /* SMROS_POSIX_SCHED_H */
