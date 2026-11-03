export interface ContactFormData {
    name: string
    email: string
    message: string
}

export interface ContactResponse {
    success: boolean
    reply_message?: string
}

export function validateContactForm(data: ContactFormData): string | null {
    if (!data.name.trim()) return 'Please enter your name.'
    if (data.name.length > 50) return 'Name is too long (50 characters max).'
    const email = data.email.trim()
    if (!email.includes('@') || !email.includes('.') || email.length < 5 || email.length > 254) {
        return 'Please enter a valid email address.'
    }
    const message = data.message.trim()
    if (message.length < 100) return 'Please add a little more detail (at least 100 characters).'
    if (message.length > 5000) return 'Message is too long (5,000 characters max).'
    return null
}

const CONTACT_SUBMISSION_COOKIE = 'contact-submission'
const CONTACT_COOLDOWN_MS = 60 * 60 * 1000

export function getContactCooldownRemaining(): number {
    if (typeof document === 'undefined') return 0

    const cookie = document.cookie
        .split('; ')
        .find((entry) => entry.startsWith(`${CONTACT_SUBMISSION_COOKIE}=`))
    const submittedAt = Number(cookie?.split('=')[1])
    const remaining = CONTACT_COOLDOWN_MS - (Date.now() - submittedAt)

    if (!cookie || !Number.isFinite(submittedAt) || remaining <= 0) {
        document.cookie = `${CONTACT_SUBMISSION_COOKIE}=; Max-Age=0; Path=/; SameSite=Lax`
        return 0
    }

    return remaining
}

export function setContactCooldown(): void {
    document.cookie = `${CONTACT_SUBMISSION_COOKIE}=${Date.now()}; Max-Age=3600; Path=/; SameSite=Lax`
}

export async function submitContactForm(data: ContactFormData): Promise<ContactResponse> {
    const response = await fetch('/api/contact', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
    })

    const contentType = response.headers.get('content-type') ?? ''
    if (!contentType.includes('application/json')) {
        throw new Error(`Contact endpoint returned ${response.status} instead of JSON`)
    }

    const result = await response.json() as ContactResponse
    if (!response.ok) {
        throw new Error(result.reply_message ?? 'Unable to send message')
    }

    return result
}
