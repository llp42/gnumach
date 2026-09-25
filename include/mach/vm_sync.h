/*
 * Copyright (c) 2018 Free Software Foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 2 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 * All Rights Reserved.
 */
/*
 * Copyright (c) 2026 Leonardo Lopes Pereira <leonardolopespereira@outlook.com>
 */

#ifndef	_MACH_VM_SYNC_H_
#define	_MACH_VM_SYNC_H_

/*
 *	Types defined:
 *
 *	vm_sync_t		VM synchronization flags
 */

typedef int		vm_sync_t;

/*
 *	Synchronization values
 */

#define	VM_SYNC_ASYNCHRONOUS	((vm_sync_t) 0x01)
#define	VM_SYNC_SYNCHRONOUS	((vm_sync_t) 0x02)
#define	VM_SYNC_INVALIDATE	((vm_sync_t) 0x04)

#endif	/* _MACH_VM_SYNC_H_ */
